# Handover to M5 — FBs, OOP, Aggregates, Strings, and Standard Library

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-10-24; finalized after M4.5 completion 2026-10-25  
Prepared after M4 (LLVM IR core codegen) completion

Read this together with the project plan (`Specification/00-Project-Plan.md` §8, M5–M7),
`Specification/Architecture/Architecture.md` §2–§6,
`Specification/Architecture/Runtime-and-Execution.md` §1–§4,
`Specification/Architecture/IO-and-Devices.md` §2–§5,
`Specification/Architecture/State-and-Retain.md` §2–§7,
`Specification/Architecture/Testing-and-Verification.md` §1–§4 and §6,
`Specification/AST/AST.md` §5–§8, `Specification/Decisions.md` D-059–D-127,
`Specification/Architecture/Online-Change-and-GOT.md`,
`Specification/Architecture/GOT-Artifacts.md`,
`Specification/Architecture/Debug-and-Monitoring.md`,
`M4-Plan.md`, `M4.5-Plan.md`, `update_plan_got.md` §13.0, and `handover_to_M4.md` (the M3→M4
input, retained for history).

## 0. Online-change addendum — final M4.5 handoff (2026-10-25)

**M4.5 is complete and M5 starts on the host/module model.** Do not implement M5 on M4's
artifact model. The resolved online-change architecture supersedes several M4-era
recommendations in this document:

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
- `stc build` has `--emit=module` and the paired GOT manifest. M5 must emit identity records for
  every new root/field, function/method/FB body, type/descriptor, instance, task, and export.
- `exe` is a thin launcher and `lib` is the embedding host library with generated code kept in
  the separate module; the two may be emitted together (D-109–D-112). M4-era statements that
  `exe` and `lib` cannot coexist describe the historical M4 path.
- M5 exit criteria include hot-swap coverage for FB/class state, aggregates/strings, interfaces,
  descriptors, and `POINTER TO`; see §2 and the project plan M5.

### 0.1 Implemented M4.5 contracts

M4.5 phases 0–10 are complete. The implementation is the contract M5 extends:

| Crate | Implemented contract |
| --- | --- |
| `stc-got` | Stable keys and `GotId`; catalog records; binary/canonical-JSON manifest and plan format 1; descriptor format-1 codec, digest, and pairing; baseline matching; tracked rename/move; preserve/allocate/drop/reject classification; canonical slot assignment; build-ID derivation |
| `stc-codegen::ir` / `verify` | GOT slot-table access, cell-kind checks, indirect function/import calls, ABI 2 entry points, descriptor/function-table globals, structural verification |
| `stc-codegen::module` | M4 scalar-surface root catalog (global, program field, direct, task guard), instance/task/export/function records, canonical slot bindings, descriptor, manifest pair, online-change plan, deterministic snapshots |
| `stc-codegen` lowering | Persistent scalars, program fields, direct `AT` variables, retained leaves, and task guards through GOT cells; user/program calls through function cells; host services through import cells; retain helpers and export wrappers |
| `stc-runtime` | Host GOT, stable zeroed roots, quarantine, transient allocator, descriptor/manifest/plan validation, platform loader, ABI 2.0 host services, atomic manifest store/download, exact-baseline load/prepare/activate/commit/rollback/unload, generation pins, process-image reconfiguration/remap, runtime warnings, non-local trap boundary |
| `stc-driver` | `module`/`exe`/`lib`/`metadata` emission; `--got-manifest`, `--cold`, `--rename`, `--allow-untracked-removal`; adjacent module/manifest/metadata naming |

Tests cover identity and second-build reuse, golden descriptors/manifests, byte-identical double
builds, host lifecycle and rollback, real compiler-produced module hot swaps (code-only, add,
drop, tracked rename, untracked-removal policy, type/layout rejection with cold reset), direct
remap, malformed/tampered artifacts, fuzzed manifests/descriptors, concurrent pins, launcher and
embedding-host artifact loading, and the recorded O(1) switch measurement. CI runs the `online`
suite on Windows x64 and Ubuntu x64.

### 0.2 What M5 must extend

- Extend the **catalog projection** in `stc_codegen::module` for aggregate/FB/class/interface
  roots, methods, vtable/type descriptors, and place descriptors. Do not derive identities in a
  second path.
- Use `stc-got` classification and slot assignment unchanged; format 1 leaves the later record
  tables empty, so M5 populates records rather than changing formats.
- Apply the **per-field root** model recursively. Contiguous objects, module-owned vtables, raw
  `this` pointers, and persisted references are not permitted.
- Extend `stc_codegen::ir`/`verify` for aggregate loads/stores and other M5 IR needs; indirect
  calls and `call_ptr` already exist. Keep the structural contract (no direct persistent root or
  user-callable reference).
- Extend the runtime `image`/`memory`/`host` substrate for any new host-owned state; preserve
  `Reset`/`Migrate` as reserved and keep scalar rollback semantics.
- Add tests at the same layers: catalog/manifest snapshots, clang acceptance, execution through
  module pairs, and real hot swaps.

The phase-8 preservation policy is the M5 baseline: matching roots preserve by `GotId`, new roots
initialize once, removed roots quarantine, tracked renames execute, untracked removals require an
explicit policy, incompatible type/layout changes are rejected until a cold rebuild, and the
process image/direct cells remap with a rollback snapshot.

Read [Online Change and GOT](Specification/Architecture/Online-Change-and-GOT.md),
[GOT Artifacts and Schemas](Specification/Architecture/GOT-Artifacts.md), and `M4.5-Plan.md`
before starting M5. If a recommendation below conflicts with those documents, the online-change
specification and the schema document win.

## 1. State at handover

M4.5 is implemented and complete as of 2026-10-25 (phase 9 commit `5b5da2c` plus this phase-10
documentation pass). M4 was `4c55d71` (`M4: LLVM IR core codegen`); M3 is `4ddd5ea`; M2 is
`24da2f7`; M1 is `51cab11`.

| Item | State |
| --- | --- |
| Workspace | `stc-span`, `stc-diag`, `stc-lexer`, `stc-syntax`, `stc-parser`, `stc-hir`, `stc-sema`, `stc-codegen`, `stc-got`, `stc-runtime`, `stc-driver`; `fuzz/` is separate |
| Toolchain | Rust 1.99.1, edition 2024; workspace `unsafe_code = "forbid"`; `stc-runtime` is the documented exception |
| Tests | 553 passing (`cargo test --workspace --locked`) across 58 test binaries, including codegen, clang verification, execution, host lifecycle, hot-swap, launcher/embedding, and C-harness tests |
| Checks | clippy `-D warnings`, `cargo fmt --check`, `check_docs.py`, Markdown lint all pass |
| Fuzzing | `fuzz_lexer`, `fuzz_parser`, `fuzz_analyzer`, `fuzz_codegen`, `fuzz_got_manifest`, `fuzz_module_descriptor`; descriptor/manifest targets never load native code |
| Host tools | clang/LLVM 23.1.4, `llvm-ar`, host triple `x86_64-pc-windows-msvc` |
| Artifacts | `.ll`, `.o`, loadable `<name>-<build-id>.dll`/`.so` module, `.stc-got.json`/`.stc-got.bin` manifest pair, `.stc-meta.json`, thin `.exe` launcher, embedding `.lib`/`.a` host |
| Build options | `--emit=ir,obj,module,exe,lib,metadata`, `--got-manifest`, `--cold`, `--rename`, `--allow-untracked-removal` |
| Uncommitted | phase-10 documentation only |
| Manifest | implemented; descriptor/manifest pairing, exact-parent baseline, rename/classification, and export/download work |

M4.5 delivered the online-change foundation: deterministic module IR and GOT slot access, the
root catalog and identity/manifest projections, the persistent host with stable roots and
quarantine, the module loader and swap state machine, ABI 2 host services, the execution
conversion of the M4 scalar surface, the scalar compatibility/remap/rollback policy, the thin
launcher and embedding host artifacts, artifact fuzzing, and the Windows/Linux CI suite.

`stc check` and `stc build` are the current end of the pipeline. `stc parse` remains single-file
and syntax-only. Semantic analysis is skipped when lexing/parsing produced errors (D-070).
`stc run`/`stc test`/`stc fmt` and the project `stc.toml` manifest remain later milestones (M6);
the GOT manifest is implemented.

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
M4/M4.5's explicit deferrals are the M5 work list.

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
| Build | `--emit=module` plus the GOT manifest pair; `--got-manifest`, `--cold`, `--rename`, `--allow-untracked-removal`; extend the catalog/metadata with M5 roots | project `stc.toml`, `stc run`, `stc test`, `stc fmt` (M6) |
| Test runner | recommend deferring `stc test` execution to M6 (see §9) | test discovery/runner/assert harness (M6) |

Anything outside the M5 column must fail with an explicit codegen diagnostic. Silent emission of
wrong IR remains worse than a deliberate “not implemented until M6” error.

## 3. Contracts M4.5 provides

### 3.1 Crates and dependency direction

```text
stc-span -> stc-hir -> stc-sema -> stc-parser/stc-syntax
stc-got (format/identity crate; no workspace dependencies)
stc-hir + stc-span + stc-got -> stc-codegen -> stc-driver
stc-got -> stc-runtime (staticlib; linked as an artifact, not a normal dependency)
```

Rules to preserve:

- `stc-hir` does not depend on `stc-syntax`, `stc-parser`, or `stc-sema`.
- `stc-codegen` depends only on `stc-hir`, `stc-span`, `stc-got`, and std. Tests may use
  parser/sema as dev-dependencies; production code must not reach into syntax or semantic
  internals.
- `stc-got` is the shared identity/manifest/descriptor format crate. Both the compiler and the
  runtime depend on it so there is one codec and one digest implementation.
- `stc-runtime` is not a normal Rust dependency of any workspace crate; it depends on `stc-got`
  (not on compiler crates). The driver links the runtime artifact and uses the crate only as a
  dev-dependency for host tests.
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

`stc_sema::Analysis` exposes `hir` plus the resolution map. Codegen uses only `hir`.

M4/M4.5 added these HIR fields used by M5:

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
  external targets. M4.5 binds that pseudo-`this` to the current instance's field-storage array
  while lowering the module cycle.
- `ConfigInitHir` may carry an `AT` override. Codegen rejects overrides explicitly; M5 or M6 may
  implement them.
- `PlaceHir::InstanceField` is used by `VAR_ACCESS` and other configuration paths; M4.5 supports
  only scalar/final-`AT` paths.

### 3.6 Codegen public API

```rust
pub enum Mode { Executable, Library }

pub struct ModuleBuildOptions<'a> {
    pub compiler_version: String,
    pub source_fingerprint: Digest32,
    pub baseline: Option<&'a Manifest>,
    pub cold: bool,
    pub allow_untracked_removal: bool,
    pub renames: Vec<Rename>,
}

pub struct Options<'a> {
    pub artifact_name: &'a str,
    pub target: Option<&'a str>,
    pub debug: bool,              // accepted; debug info is M6.5
    pub emit_metadata: bool,
    pub mode: Mode,
    pub relaxed: bool,
    pub release: bool,            // artifact-form flag; --release CLI is M6.5
    pub sources: &'a [String],
    pub module: ModuleBuildOptions<'a>,
}

pub struct Output {
    pub module: String,
    pub metadata: Metadata,
    pub spans: Vec<SpanEntry>,
    pub got: ModuleArtifacts,     // descriptor, manifest, slots, JSON/binary bytes
}

pub enum CodegenError {
    Unsupported { code: &'static str, message: String, span: Span },
    Internal { message: String },
}

pub fn generate(hir: &HirCrate, options: &Options<'_>) -> Result<Output, Vec<CodegenError>>;
pub fn generate_launcher(artifact_name: &str, module_file: &str, target: Option<&str>) -> String;
```

`stc_codegen::module` exposes the artifact projection M5 extends:

```rust
pub struct ModuleOptions<'a> { /* target, module name, compiler version,
                                 source fingerprint, release, relaxed, baseline,
                                 cold, allow_untracked_removal, renames */ }
pub struct ModuleArtifacts { /* catalog, manifest + binary/JSON bytes, descriptor +
                                 bytes/digest, slots, diagnostics */ }
pub fn project(hir: &HirCrate, sources: &[String], options: &ModuleOptions<'_>)
    -> Result<ModuleArtifacts, Vec<ModuleError>>;
```

`stc_codegen::ir` is the textual LLVM builder and `stc_codegen::verify` is the structural
verifier M5 must keep passing.

M4.5 `generate` sequence:

1. `hir.check_invariants()`.
2. `module::project` resolves the selected plan, builds the scalar catalog, matches the baseline,
   classifies actions, assigns canonical slots, and emits the descriptor, manifest pair, and
   online-change plan.
3. `unit::generate` lowers module-shaped IR: GOT slot access, indirect calls, entry points,
   descriptor/function-table globals, initializers, retain helpers, and export wrappers.
4. The structural verifier checks the module contract; metadata and spans are assembled and the
   IR is rendered deterministically.

Driver mapping: `Unsupported` with a span becomes a source diagnostic; empty-span errors are
printed without a fake location; `Internal` prints `STC-E-CODEGEN-002` and exits 3.

### 3.7 Internal codegen structure

```text
crates/stc-codegen/src/
  lib.rs        API, Options/ModuleBuildOptions/Mode, CodegenError, Output, SpanEntry
  ir.rs         textual IR builder: IrType, Value, Function, Module, GlobalDef, GOT helpers
  verify.rs     module structural verifier (GOT/cell/entry-point contract)
  module.rs     catalog projection, descriptor, manifest pair, plan, slot bindings
  launcher.rs   thin `main` launcher IR for the adjacent module
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
                GOT-based init/cycle/shutdown, entry points, main/library metadata/exports/retain
  metadata.rs   Json value/model and deterministic writers
  abi.rs        host-service IDs, ABI version, trap codes
  codes.rs      M4/M4.5 diagnostic/build code constants
```

`BodyInfo` contains parameter/local alloca maps, the result alloca and storage type, the slot
table pointer, an optional per-instance field-storage array, and the return label. `Lowerer` owns
the module, plan, layouts, image, retain, the projected `ModuleArtifacts`, symbol maps, span IDs,
and errors.

### 3.8 Module builder surface

```text
IrType = Void | Int(u16) | Float | Double | Ptr | Overflow(u16)
Value  = { ty: IrType, repr: String }   // repr is a temp (%tN) or literal
SymbolAttrs { linkage, visibility, export, dso_local } / external / internal / private /
             module_export
Function::new(name, attrs, ret, params)
  start_block / start_fresh_block / next_block_label / ensure_open
  temp / emit_value / emit / terminate
  branch / cond_branch / ret / unreachable
  alloca / alloca_typed / load / store / binop / unop / icmp / fcmp / select
  gep(pointee_text, base, indices, inbounds) / gep_bytes
  call(ret, callee, args) / call_ptr(ret, callee_cell, args)
  slot_cell(slots, n) / cell_value(cell) / slot_value(slots, n)
  extract_value / phi / param_value / render / check
Module::new(artifact, target)
  add_type(name, body) / add_global(GlobalDef) / add_constant_bytes(...)
  add_descriptor(name, bytes) / add_function_table(name, functions)
  declare(name, signature) / add_function(Function) / render / check
```

`Value.render()` includes the type for contexts that need it (`store`, `ret`, call arguments).
`binop`/`icmp`/`fcmp`/`phi` emit the type once, as LLVM requires. Names are stable and sorted.
`slot_cell`/`cell_value`/`slot_value` implement the GOT access contract and record structural
facts for `verify`.

### 3.9 Runtime ABI

`crates/stc-runtime` is a Rust `staticlib` built as the persistent host and embedding library.

```c
int32_t stc_abi_version(void);                     /* runtime-owned, value 2 */
int32_t stc_rt_execute(int32_t argc, char** argv, const char* module_path);
                                                   /* thin-launcher host loop */

int32_t stc_host_load_module(const char* path, int32_t mode); /* 0 cold, 1 online, 2 auto */
int32_t stc_host_cycle(void);
int32_t stc_host_shutdown(void);
void*   stc_host_symbol(const char* name);
int32_t stc_host_export_manifest(char* buffer, int64_t* len);
const char* stc_metadata_json(void);
/* stc_host_run and the full options struct are M6. */
```

Generated modules **do not import `stc_rt_*` symbols**. They receive the ABI 2.0 host API table
in `stc_module_attach` and call services through import cells: trap, cycle/monotonic/local
clocks, transient alloc/free, process-image base/size, retained-state load/save, and assert. The
table layout and signatures are fixed in
[GOT Artifacts and Schemas](Specification/Architecture/GOT-Artifacts.md) §8. The legacy
`stc_rt_*` helpers remain host-side utilities for the process image, clock, and retained-state
stubs; the state stubs return 5 (disabled) until M6.

### 3.10 Build bridge and tests

`stc build <file>... [--name] [--emit=ir,obj,module,exe,lib,metadata] [--out-dir] [--target]
[--clang] [--runtime] [--got-manifest <path>] [--cold] [--rename old=new]...
[--allow-untracked-removal] [--relaxed] [--message-format] [-v]`.

- Default emit is `ir`; default out-dir is `target/stc/debug`; default name is the sanitized
  first sorted file stem.
- The generated program is always the loadable module. `module` emits it with
  `<name>.stc-got.json`/`.bin` and `.stc-meta.json`; `exe` adds the thin launcher; `lib` copies
  the embedding host archive; `exe` and `lib` may be emitted together, and generated code is
  never linked into either host artifact.
- `--got-manifest` selects the identity baseline, `--cold` ignores it, and `--rename` records a
  tracked rename/move. A missing/stale/invalid baseline warns and produces a cold-layout plan.
- Windows MSVC link discovery lives in `crates/stc-driver/src/toolchain.rs`; Linux GNU adds
  `-ldl -lpthread -lm`. The embedding `lib` artifact is the runtime staticlib copied into the
  output directory; generated code is never archived into it.
- Execution tests live in `crates/stc-driver/tests/execution.rs` and `module_cli.rs`; C harnesses
  in `crates/stc-driver/tests/harness/`. The runtime is built once per test process.
- Codegen tests live in `crates/stc-codegen/tests/`; the snapshot harness uses
  `STC_UPDATE_SNAPSHOTS=1`; clang verification is mandatory when `STC_REQUIRE_CLANG=1`.
- Runtime host tests live in `crates/stc-runtime/tests/`; the switch-latency measurement is an
  ignored test.

## 4. M4 behavior and gaps M5 must close

The gaps below are still M5 work, but the M4.5 conversion changed several implementation facts.
§4.12 lists the corrections; where this section conflicts with §0, §0 wins.

### 4.1 Preflight rejects every M5 definition

`unit.rs::preflight` reports `STC-E-CODEGEN-001` for `Struct`, `Enum`, `NamedValue`, `Subrange`,
`Class`, `FunctionBlock`, and `Interface`, and `layout::scalar_kind` rejects `Ty::Named`,
`Ty::String`, `Ty::Array`, and `Ty::Ref`. Replace this with real support and narrow the
unsupported set to genuinely out-of-profile constructs. Keep the “one definition-level error,
not a per-expression cascade” rule (D-093).

### 4.2 The IR type system is scalar-only

`IrType` has no named, array, struct, string, reference, or interface representation, and
`Module.type_defs` stores raw strings. M5 needs a typed representation that still renders
deterministically. Recommended direction: add a module-IR type table with a small `TypeRef`
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

- Per-field host roots for every persistent class/FB field, including inherited fields, with
  object handles and runtime-owned records/descriptors (not a named module struct with raw
  `this`).
- FB bodies called through function cells with the instance handle/field-storage view, with
  input fields written before the body and output fields read after it.
- Method functions called through function cells with the receiver handle first.
- Prepare-time initialization of fresh FB/class fields and nested instances; preserved roots are
  never reinitialized.
- Runtime-owned descriptor/vtable records with deterministic slot order from HIR.
- Interface values `{handle, descriptor}` and dispatch through host-owned function cells.
- `THIS`/`SUPER` places and calls, including `SUPER()` body invocation, on the same handle.

### 4.5 Calls and places deferred to M5

M4.5 lowers only `CalleeHir::Function` and `CalleeHir::Conversion` (through function cells); all
other callee kinds are rejected and remain M5. M4.5 also rejects `PlaceHir::Index`, `Deref`, `Direct` as ordinary storage, and
`InstanceField` paths beyond a final `AT` leaf. M5 must implement:

- `StdFunction` (all categories in Runtime §3.1).
- `Method`, `SuperMethod`, `InterfaceMethod`, `Fb`, `SuperBody`.
- `Ref`, `Null`, `Deref`, `RefAssign`, `Attempt`, `?=`.
- `ArrayInit`, `StructInit`, string literals/copies, aggregate assignment and comparison.
- Output arguments (`=>`, `ArgHir.target`/`negated`) and `VAR_IN_OUT` parameters.
- Arrays with declared bounds, 1-based indexing, bounds traps, and conformant descriptors.

M4.5 has a working trap path (`trap_if`, span IDs, the host trap boundary) and checked
arithmetic; reuse it for bounds,
string capacity, subrange assignment, and conversion failures.

### 4.6 Retain is scalar-only

`RetainLayout` serializes scalar retained globals and program fields as a packed sequence of
leaves sorted by path. The leaves are copied from host roots, not module globals. M5 must extend
it to:

- structs, arrays, strings, enums/subranges (as base integers), and FB/class instance state;
- nested paths and inheritance (`Main.Timer.Elapsed`);
- exclusion of references, interface pointers, and vtable/type-descriptor pointers;
- recursive leaf flattening whose payload offsets are packed sizes, not target struct offsets.

Do **not** compute LLVM struct offsets in the compiler. Save/load should use GEP chains per leaf
and write packed payload bytes; metadata offsets remain payload offsets (D-087). Non-persistable
leaves must keep `STC-E-RETAIN-001` behavior. File-backed state remains M6: keep the
`stc_retain_*` ABI; the ABI 2 state services stay disabled until M6.

### 4.7 Exports are function-only

Sema accepts `{stc:export}` only on `FUNCTION` in M4.5. Decide whether M5 exports methods or
class/FB types (likely no: the C ABI is scalar-function-oriented). If export support expands,
keep the same validation, exact C names, and uniqueness rules. `stc_abi_version` remains
runtime-owned; do not reintroduce a duplicate generated symbol.

### 4.8 Standard functions and standard FBs are signature-only

`HirCrate.std_fns` contains signatures (`StdFnKind`, `StdParam`, `StdResult`). M4.5 rejects every
`StdFunction` call. M5 must implement all `StdFnKind` variants, including shifts/rotates,
selection, comparisons, math, date/time, and string functions. The standard FB definitions are
synthetic empty-span HIR defs with fields but no bodies; codegen currently skips all empty-span
defs. M5 must provide real FB bodies, not emit empty functions.

Recommended delivery: add `stdlib/*.st` sources for `SR`, `RS`, `R_TRIG`, `F_TRIG`, `CTU`,
`CTD`, `CTUD`, `TP`, `TON`, `TOF` and inject/compile them as production HIR in the driver or
sema. If textual stdlib injection is deferred, synthesize HIR bodies and record the deviation.
Timers must read the host's latched `cycle_now_ns` service, never the live clock.

### 4.9 Clock and tasks

The runtime already latches the clock per cycle and exposes it through the ABI 2 `cycle_now_ns`
service; library hosts can set the cycle time through the `stc_rt_set_cycle_time_ns` host utility.
Generated code does not yet read the clock because there are no time functions or timers; M5 adds
those reads. M4.5 still accepts at most one task per resource (`STC-E-RUN-001`) and runs all
instances each cycle; a `SINGLE` task uses a first-cycle guard. Full task scheduling, priorities,
and a mock-clock CLI remain M6.

### 4.10 Test-only units and the test runner

M4.5 codegen skips `Def.test_only` defs and treats `{stc:test}` functions as production-absent
(D-071). The M5 plan lists a “test runner MVP”, but test discovery depends on the M6 manifest.
Recommend deferring `stc test` to M6 and continuing to use `cargo test` plus C harnesses in M5.
If a file-list MVP is required, scope it explicitly in an M5 decision and do not pull manifest
rules forward.

### 4.11 Other HIR/codegen facts M5 will meet

- `Ty::Named` includes programs; a `PROGRAM` field access in a program body uses
  `PlaceHir::This(program)` internally even though the `THIS` keyword is rejected there.
- `VAR_EXTERNAL` aliases are lowered to `PlaceHir::Global(target)`; do not allocate storage.
- Missing defaulted input arguments are absent from `ArgHir`; M4.5 materializes scalar defaults
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
- Codegen rejects `VAR_CONFIG AT` overrides; do not silently ignore them.
- `VAR_ACCESS` is resolved in HIR but its metadata is M6; do not invent code for it.

### 4.12 M4.5 corrections to the M4 facts above

- Persistent globals and program fields are **host GOT roots**, not module globals. Program
  bodies receive a per-instance `[N x ptr]` field-storage array; scalar layout/config code still
  computes field order and types, but storage lives in the host.
- User functions, program bodies, and exports are reached through **function cells**; a symbol
  table still exists for module-local helpers, export wrappers, and the function-address table,
  but emitted calls use `call_ptr` through a cell.
- The generated module exports the seven ABI 2 `stc_module_*` entry points, descriptor, function
  table, retain helpers, and export wrappers. `stc_init` is an internal prepare helper, not an
  exported ABI-1 entry point.
- `stc build` always produces a loadable module. `exe` is a thin launcher, `lib` is the embedding
  host archive, and they may be emitted together; generated code is never linked into either.
- Generated modules do not import `stc_rt_*`. Traps, image access, retained state, and any
  clock/allocation/assert use go through ABI 2 host API import cells (D-116, D-124).
- Retained leaves are host roots; online prepare initializes only fresh roots, and the
  file-backed state services remain disabled until M6.
- Task guards are GOT roots; task execution state is host-owned and keyed by task identity.
- Direct `AT` variables are `Direct` roots with per-generation cells. The host reads the
  metadata sidecar to bind them and remaps values by `GotId` at activation (D-125).
- Metadata carries the module/manifest/descriptor summary, the process image and direct
  bindings, and scalar retain leaves. It is not the M4 metadata schema; see CLI and Manifest §4.
- Trap handling crosses a non-local host boundary. M5 code must keep generated functions
  `uwtable` and must not assume it can resume after a trap.

## 5. Recommended M5 architecture

The recommendations below remain a useful starting point, but the representations are for
**transient values only**. Persistent aggregates, objects, interfaces, and references must use
the §0/Online Change model: recursive per-field roots, stable handles, runtime-owned
descriptors/vtables, and place descriptors. Where this section proposes module-owned
descriptors/vtables or fat pointers for persistent-capable values, §0 and the online-change
specification win.

### 5.1 Type and layout layer

Add a `TypeLayout`/`StorageType` abstraction in `layout.rs` (or a new `types.rs`):

```rust
pub enum StorageType {
    Scalar(ScalarKind),
    Named(u32),              // index into a module type table
    Array { element: Box<StorageType>, count: u64 },
    String { kind: StringKind, capacity: u32 },
    Ref,                     // transient view; persistent REF_TO uses a place descriptor
    Interface,               // transient {ptr, ptr}; persistent values use {handle, descriptor}
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

- Reuse the sema-assigned `vtable` order. Vtables and type descriptors are **runtime-owned**
  records (D-098); the module emits catalog/descriptor records and slot bindings, not
  module-owned vtable globals that would dangle after unload.
- Interface dispatch loads a host-owned descriptor/vtable cell and calls the method's
  **function cell** through `call_ptr`; a raw `call_indirect` on a module-owned address is not
  permitted for persistent-capable values.
- `SUPER.method()` calls the base implementation through its function cell.
- `SUPER()` invokes the base FB body on the current instance through its function cell.
- FB invocation:
  1. evaluate the instance/handle place;
  2. store input arguments into the host-rooted input fields (including nested/defaulted inputs);
  3. call the FB body function cell with the instance handle/field storage;
  4. copy output fields to `ArgHir.target` places, applying `negated` for `NOT =>`.
- Method signatures: the receiver handle/field storage first; scalar/aggregate parameters follow
  the chosen ABI.
- `stc_module_prepare` initializes fresh instance roots and declared initializers; preserved
  roots are never reinitialized. Keep descriptor/vtable records in the host, not patched into
  module text.
- `THIS` is the current instance handle; `SUPER` uses the base view of the same handle.
- Register every method/FB body/descriptor as an identity in the catalog; do not create
  identities for compiler helpers.

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
- Standard functions must never call the live monotonic clock; `NOW()`/`TIME()` use the host
  `cycle_now_ns` service.
- Add Lean-generated vector tests for exact functions, or keep them as test vectors under the
  existing `tests/vectors` convention if present.

### 5.6 Standard FB library and timers

- Compile the standard FBs from `stdlib/*.st` into production HIR, or synthesize equivalent
  HIR bodies. They live in the reserved `STC_STD` namespace and are shadowable.
- Implement `SR`, `RS`, `R_TRIG`, `F_TRIG`, `CTU`, `CTD`, `CTUD`, `TP`, `TON`, `TOF`.
- Timers capture start times in instance fields and compare against the latched `cycle_now_ns`
  service. All timers in one scan see the same value.
- Provide deterministic tests using `stc_rt_set_cycle_time_ns` in embedding-host harnesses or
  `--cycle-time` once the runtime CLI supports it (M6).
- Do not implement multi-task scheduling in M5; keep the M4.5 one-task rule and metadata task
  attributes.

### 5.7 Retained state

- Flatten aggregate/string/FB instance state into scalar/string leaves with readable paths,
  rooted in the host per-field root registry.
- Payload offsets are cumulative packed sizes in sorted-path order (D-087); no LLVM struct
  offsets.
- Exclude pointers, interface/vtable/descriptor pointers, references, and `VAR_IN_OUT`.
- Extend metadata `retain.leaves` with aggregate-derived paths and canonical type names; the
  owning root `GotId` stays in the GOT manifest.
- Keep `stc_retain_size/layout_hash/save/load`; save/load use GEP chains + `align 1` stores over
  host roots. Online prepare initializes only fresh roots and skips the state file; cold installs
  keep the M4.5 reset policy.
- Update D-087/D-075 to record the M5 aggregate scope; file I/O remains M6.

### 5.8 Metadata and ABI

- Extend `metadata.rs` with new metadata if needed (types/vtables are not required by the
  current specs; prefer not to invent keys). The M4.5 schema is in CLI and Manifest §4.
- If host services are added, append them to the ABI 2 table and bump the minor version per
  Runtime §2.2; record it in Decisions. `stc_abi_version` stays runtime-owned.
- Keep `stc_metadata_json` embedding-host-only; executable metadata remains the sidecar JSON.
- Span/trap tables stay sorted by `(FileId, start, end)`; trap messages still carry only the
  span ID in M5.

### 5.9 Driver

The M4.5 build bridge already emits the module/manifest pair and accepts the baseline options;
M5 extends the projected catalog and lowering, not the artifact pipeline. Possible additions:

- include `stdlib/*.st` in the analyzed file set (deterministic order; reserved package);
- optionally add `--stdlib <dir>` for experiments, but keep the default self-contained;
- keep `--emit` names, module/manifest naming, and exit codes stable.

The project `stc.toml` manifest and dependency graph remain M6; use a driver-level convention
for stdlib source lists in M5.

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

1. Record M5 decisions starting at D-128 and write `M5-Plan.md`; M4.5 is complete and committed.
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

Record each in `Specification/Decisions.md` before or as it is implemented. M4.5 already fixes
several boundaries: item 2 extends the existing `stc-got` catalog/type-key/layout model rather
than designing a second type system; item 5's place-descriptor and nullable-interface
representation is fixed by D-097; item 6's runtime-owned descriptors/vtables and cell dispatch
are fixed by D-098; item 11's retained leaves are host roots; item 12's host API is append-only
ABI 2; item 16's determinism baseline is the GOT artifact model; and item 17's CI already
includes the `online` job. The concrete M5 record contents and trap/ABI choices remain open.

1. **Aggregate ABI.** Which aggregates pass by value, by pointer, or result via hidden pointer;
   how arrays/strings/FB instances are passed; whether aggregate function results are allowed.
2. **Module type system.** Extend the catalog/type-key/layout records for M5 types; type
   table/ID design and named-type rendering in module IR; how operand types are checked without
   `Copy`-hostile recursive enums.
3. **Aggregate layout.** Prefix inheritance, `OVERLAP`, arrays, strings, conformant descriptors;
   no target datalayout.
4. **Bounds and assignment checks.** Subrange assignment, array bounds, string capacity,
   conversion ranges; which trap codes.
5. **References and assignment attempt.** `REF`/`NULL`/deref semantics, place-descriptor
   conversion, `?=` helper/descriptor format, null traps.
6. **OOP representation.** Descriptor/vtable record contents and slot order source (HIR
   `vtable`), function-cell dispatch rules, interface upcast/downcast.
7. **FB invocation.** Input/output binding order, defaults, negation, nested instances,
   reentrancy/aliasing.
8. **Standard functions.** Inline vs runtime split, math library calls, Lean vector coverage,
   date/time conversion rules.
9. **Standard FB delivery.** `stdlib/*.st` injection vs synthesized HIR; reserved namespace;
   shadowing and diagnostics.
10. **Clock and timers.** Library mock-clock usage, duration arithmetic, timer state fields,
    `STC_CLOCK_ADVANCE` scope.
11. **Retained scope.** Aggregate/string/FB leaf flattening over host roots, packed offsets,
    non-persistable exclusions, hash-format compatibility.
12. **Host API additions.** Append `stc_rt_str_*`/`stc_rt_math_*`/attempt services to ABI 2;
    bump the minor version; normalize `i64` vs `uint64_t` length declarations.
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
- [ ] `stc build` still emits a deterministic loadable module, paired manifest, metadata, thin
      launcher, and embedding host; `clang` accepts and runs the module on the host.
- [ ] Unsupported M6 constructs (project `stc.toml`, `stc run/test`, device tree, file-backed
      state, full task scheduling) produce explicit diagnostics or remain absent as documented.
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
cargo run -p stc-driver -- build examples/hello.st --emit=module,metadata
cargo run -p stc-driver -- build examples/hello.st --emit=module --got-manifest active.stc-got.json
cargo run -p stc-driver -- build examples/hello.st --emit=exe,lib,metadata \
    --runtime target/debug/stc_runtime.lib

# IR verification
clang -x ir -c -o /tmp/hello.o target/stc/debug/hello.ll

# Fuzz workspace
cargo check --manifest-path fuzz/Cargo.toml --locked --offline
cargo +nightly fuzz run fuzz_got_manifest -- -max_total_time=60 -rss_limit_mb=2048 -timeout=25
cargo +nightly fuzz run fuzz_module_descriptor -- -max_total_time=60 -rss_limit_mb=2048 -timeout=25
rustfmt --edition 2024 --check fuzz/fuzz_targets/*.rs
```

Runtime artifact names differ by host: `stc_runtime.lib` on Windows MSVC,
`libstc_runtime.a` on Linux GNU. Use the test/driver discovery helpers rather than hard-coding
paths.

In PowerShell, set environment variables first, for example
`$env:RUSTFLAGS = "-D warnings"; cargo clippy --workspace --all-targets --locked -- -D warnings`.
Set `$env:STC_REQUIRE_CLANG = "1"` to make clang verification mandatory locally.

## 12. Key files after M4.5

- `crates/stc-got/src/{lib,catalog,model,manifest,descriptor,identity,build,...}.rs` — stable
  keys, `GotId`, catalog, manifest/plan codecs, descriptor format 1
- `crates/stc-codegen/src/{lib,ir,verify,module,launcher,layout,mangle,config,expr,stmt,unit,metadata,abi,codes}.rs`
- `crates/stc-codegen/tests/` — builder/verifier, module projection, golden manifest/descriptor,
  clang acceptance, determinism
- `crates/stc-runtime/src/{lib,got,memory,catalog,migration,loader,host_api,store,host,image,module_info,trap_boundary}.rs`
- `crates/stc-runtime/tests/{host_lifecycle,image_remap,switch_latency,...}.rs` — swap,
  preservation/remap, pins, warnings, latency
- `crates/stc-driver/src/{main,build,toolchain,lib}.rs` — CLI, module/launcher/embedding
  artifacts, link discovery
- `crates/stc-driver/tests/{execution,module_cli,...}.rs` and `tests/harness/*.c` — module
  execution, hot-swap CLI, launcher/embedding C harnesses
- `crates/stc-hir/src/{hir,ty,id,interner,dump}.rs` — resolved HIR and invariants
- `crates/stc-sema/src/{session,collect,types,resolve,consteval,check,prelude,codes}.rs`
- `crates/stc-sema/tests/{conformance,traceability,snapshots,hir_lowering,m4_gaps}.rs`
- `fuzz/fuzz_targets/{fuzz_got_manifest,fuzz_module_descriptor}.rs`
- `Specification/Decisions.md` D-059–D-127; `M4.5-Plan.md`; `M4-Plan.md`;
  `handover_to_M4.md`
- Architecture §4 (especially §4.3–§4.10), Runtime §1–§4, IO §2/§5, State §2/§5–§6,
  Testing §1–§4/§6, GOT Artifacts §7–§12, Debug and Monitoring §12, AST §5–§8
