# M4 Implementation Plan — LLVM IR Core Codegen

Profile: `stc-2013-oop-utf8-v1`
Date: 2026-10-24
Inputs: `handover_to_M4.md`, `Specification/00-Project-Plan.md` §8,
`Specification/Architecture/Architecture.md` §2–§6,
`Specification/Architecture/Runtime-and-Execution.md` §1–§4,
`Specification/Architecture/IO-and-Devices.md` §2–§5,
`Specification/Architecture/State-and-Retain.md` §2–§7,
`Specification/Architecture/Testing-and-Verification.md` §1–§4 and §6,
`Specification/AST/AST.md` §5–§8, and D-059–D-075.

M3 is committed (`4ddd5ea`) and the tree is clean. The M4 diff starts from the M3 handover.

## 1. Deliverables

| Area | Deliverable |
| --- | --- |
| Crate `stc-codegen` | HIR-only textual LLVM IR builder, scalar layout, mangling, expression/statement/unit lowering, configuration/metadata, retain layout |
| Crate `stc-runtime` | Rust `staticlib` with the C ABI, process-image buffers, cycle clock, traps, retained-state stubs |
| HIR | `{stc:io}` hints, validated export names, folded task values, LDATE unit fix |
| Driver | `stc build <file>...` with `--emit=ir,obj,exe,lib,metadata`, clang/runtime discovery, deterministic artifacts, exit codes 0/1/2/3 |
| Diagnostics | `STC-E-CODEGEN-*`, `STC-E-BUILD-*`, `STC-E-EXPORT-*`, `STC-E-RUN-002` |
| Tests | Codegen unit/snapshot/determinism/deep-chain tests, clang IR verification, execution tests, library/C harness tests |
| Fuzzing | `fuzz_codegen` smoke target |
| Documentation | D-076–D-093, status updates in README/Decisions/project plan/conformance/CLI/runtime/IO/state/testing |

M4 proves the HIR-to-IR, clang, runtime, and ABI path on elementary scalar code. FBs, OOP,
aggregates, strings, references, standard functions, the standard FB library, manifest/project
discovery, `stc run`, and `stc test` remain later milestones.

## 2. Crate shape

```text
crates/stc-codegen/
  src/lib.rs        public generate() API, options, errors, output
  src/module.rs     textual module/function/block/value builder
  src/layout.rs     scalar mapping, sizes, canonical type names, process image, retain layout
  src/mangle.rs     stable names and FNV-1a signature hashes
  src/expr.rs       expression lowering, scalar operators, checked arithmetic, casts
  src/stmt.rs       statements and control flow
  src/unit.rs       functions, programs, globals, stc_init/cycle/shutdown/main
  src/config.rs     configuration selection, instances, bindings, task guard
  src/metadata.rs   deterministic JSON model and writer
  src/abi.rs        runtime declarations and ABI constants
  src/codes.rs      codegen/build diagnostic codes
crates/stc-runtime/
  Cargo.toml        crate-type = ["staticlib"]
  src/lib.rs        C ABI, image, traps, clock, state stubs
```

Dependency direction:

```text
stc-span -> stc-hir -> stc-codegen -> stc-driver
stc-runtime (independent staticlib, linked by the driver)
```

`stc-codegen` depends only on `stc-hir` and `stc-span`; dev-dependencies on parser/sema are
limited to tests. `stc-hir` never depends on syntax/parser/sema. `stc-runtime` is not a Rust
dependency of any workspace crate.

## 3. HIR gaps closed first

1. **I/O hints.** Add `IoHint { device, channel, direction, payload, span }` and
   `IoDirection` to `stc-hir`. Add `VarDecl::pragmas()` to `stc-syntax`; populate
   `FieldHir.io` and `DefKind::Global.io` from the parser-validated `{stc:io}` payload with
   canonical JSON text.
2. **Export names.** Add `Def.export_name: Option<Symbol>`. Validate in sema: `{stc:export}`
   only on `FUNCTION` in M4; `{stc:no_mangle}` requires export; no-mangle names are C
   identifiers, unique across exports/globals, and outside `stc_*`/`llvm.*`. Non-no-mangle
   exports record `stc_export_<normalized>`.
3. **Task values.** Add `single_value: Option<bool>`, `interval_ns: Option<i128>`, and
   `priority_value: Option<i128>` to `TaskHir`, folded from the checked HIR constant
   expression. Sema continues to diagnose non-constant/mistyped attributes.
4. **Date units.** LDATE literals are encoded in nanoseconds (profile §243); consteval and
   casts between DATE/LDATE convert units instead of reinterpreting days.
5. **Configuration choice.** Codegen selects exactly one configuration, or synthesizes
   `_default` for a single program; zero/multiple candidates produce `STC-E-RUN-002`.
   More than one task per resource is `STC-E-RUN-001`.

## 4. M4 supported surface

| Area | Supported | Rejected as `STC-E-CODEGEN-001` |
| --- | --- | --- |
| Types | `BOOL`, signed/unsigned `8/16/32/64`, bit strings, `REAL`/`LREAL`, TIME/LTIME, DATE/LDATE/TOD/LTOD/DT/LDT, CHAR/WCHAR/UCHAR | enums, subranges, named-value types, structs, arrays, strings, references, classes, FBs, interfaces |
| Expressions | literals, scalar places, bit-field places, direct variables, binary/unary scalar operators, implicit/explicit casts, user function calls, conversion calls | std-function calls, method/FB/interface calls, `SUPER`, `REF`/`NULL`/deref, array/struct initializers, `Error` |
| Statements | assignment, call, `RETURN`, `IF`, `CASE`, `FOR`, `WHILE`, `REPEAT`, `EXIT`, `CONTINUE`, empty | `RefAssign`, `Attempt` |
| Functions | scalar result, scalar input parameters, scalar locals | output/in-out parameters, methods, stored procedures |
| Programs | scalar fields/body, configured or implicit single instance, scalar bindings, scalar `VAR_CONFIG` inits | `VAR_CONFIG AT` overrides, FB/class state, tasks beyond one per resource |
| Globals | scalar globals, scalar constant initializers | aggregates/strings/references |
| I/O | `AT` direct variables, process-image layout/metadata, defaults written once by `stc_init` | wildcard layout/access, device tree, `VAR_ACCESS` metadata |
| ABI | `stc_init`/`stc_cycle`/`stc_shutdown`, `main`, library exports, traps, cycle clock stub, process image, scalar-leaf retained state | vtables/type descriptors, file-backed state, `stc_rt_assert` |

Preflight walks every non-synthetic definition once and reports one definition-level error per
unsupported construct. Unsupported callee kinds report at the call site. A single unsupported
feature does not cascade into per-expression errors.

## 5. IR conventions

- Textual LLVM IR, opaque pointers, no LLVM library linkage.
- Header: compiler version and profile; optional `target triple` from `--target`; never
  `target datalayout`, absolute paths, timestamps, or host names.
- Functions, globals, and declarations emitted in sorted mangled-name order; metadata JSON keys
  sorted.
- Monotonic temp counter `%tN`, stable labels `bbN`; each block has exactly one terminator.
- Floats as exact hexadecimal constants; integer constants carry explicit LLVM types.
- `BOOL` is `i8` in memory and `i1` in SSA; storage normalizes with `icmp ne 0`.
- Integer sizes: 8/16/32/64 bits; REAL/LREAL `float`/`double`; TIME/LDATE/TOD/LTOD/DT/LDT and
  LTIME `i64`; DATE `i64` days; CHAR/WCHAR/UCHAR `i8`/`i16`/`i32`.
- No hard-coded pointer size, alignment, or struct offsets; process-image/retain access uses
  byte offsets with `align 1`.
- A structural verifier (balanced labels, terminators, referenced temporaries) runs in tests;
  tests also pass modules to `clang -x ir -c`.

## 6. Lowering

### 6.1 Names

`stc_fn_<ns>_<name>_<sig>`, `stc_g_<ns>_<name>`, `stc_pi_<instance>`,
`stc_prog_<type>_body`, `stc_export_<name>`. Namespace segments join with `_n_`; the session
root `_root` is omitted. Identifiers use uppercase normalized text; characters outside the C
identifier subset escape as `_uXXXX_`. Signature hashes are FNV-1a 64 over canonical parameter
and result type names, not `TyId`s.

### 6.2 Functions

Scalar result returns directly; no result returns `void`. Input parameters pass by value. The
result place and locals are entry-block allocas initialized from `LocalHir.init`/parameter
defaults/type defaults. `RETURN` branches to a shared return block. Missing defaulted input
arguments materialize the parameter's literal default at the call site.

### 6.3 Programs and units

A program type gets a named LLVM struct of its non-`AT` scalar fields. Each instance is a global
`@stc_pi_<instance>`. The body is `void @stc_prog_<type>_body(ptr %this)`; `This(program)` is
`%this` and bare fields are GEPs. `stc_init` zero-initializes globals/instances and applies
declared and `VAR_CONFIG` initializers. `stc_cycle` applies input bindings, runs each instance in
HIR order, then applies output bindings. A `SINGLE` task runs its programs only on the first
cycle through a generated `i1` guard. `stc_shutdown` runs the retained-state save path.
Executable mode adds `main` calling `stc_rt_execute`; library mode exports `stc_init`,
`stc_cycle`, `stc_shutdown`, `stc_abi_version`, `stc_metadata_json`, and export wrappers.

### 6.4 Operators and checks

- `+`/`-`/`*` on integers use `llvm.*.with.overflow` and trap `STC_TRAP_OVERFLOW`.
- `/`/`MOD` check zero (`STC_TRAP_DIV_ZERO`); signed `MIN / -1` traps overflow; `MOD` uses
  `srem`/`urem`.
- `**` uses LLVM `pow` for floats and a checked multiplication loop for integers; negative
  integer exponents trap `STC_TRAP_RANGE`.
- Bitwise operators use i1 for BOOL and unsigned integer ops for bit strings; `NOT` is
  `xor true` / `xor -1`.
- Comparisons use signed/unsigned predicates by type; floats use ordered predicates except
  `<>`, which is `une`. BOOL comparisons use i1.
- Unary negation checks integer/TIME minimum; floats use `fneg`.
- Float division follows IEEE (no trap, no fast-math).
- Every narrowing/real→int conversion checks range and traps `STC_TRAP_RANGE` or
  `STC_TRAP_CONVERSION`. Sema has already inserted every cast; codegen never invents one.
- Trap sites use deterministic span IDs sorted by `(FileId, start, end)`, deduplicated.

### 6.5 Control flow

`IF`/`ELSIF`/`ELSE` use one block per arm and a merge block. `CASE` uses a comparison chain for
labels and ranges. `FOR` stores start, compares against the final value using the constant step
sign, runs a checked increment, and loops; `CONTINUE` targets the increment block, `EXIT` the
exit block. `WHILE` uses a condition block; `REPEAT` runs the body then the condition.
Statements after `RETURN`/`EXIT`/`CONTINUE` start a fresh unreachable block.

### 6.6 Process image and retained state

- Areas are contiguous: `%I`, `%Q`, `%M`; offsets are absolute per area; image size is
  `ceil(max end bit / 8)`.
- Address normalization: explicit bit with two indices is `byte.bit`; no size with two indices
  is a bit; no size with one index is a byte. Wildcards are rejected when laid out.
- `AT` variables have no ordinary storage; accesses use `align 1` loads/stores (masked
  read-modify-write for bits and writable areas).
- `stc_init` calls `stc_rt_io_configure` and `stc_rt_io_image`, caches the base pointer, and
  writes input defaults once. Defaults are not re-applied by the generated cycle, so a library
  host can drive inputs between cycles; M6 owns per-cycle backend precedence.
- Scalar retained leaves (retained globals and retained fields of instances) are sorted by
  uppercase path; offsets are cumulative sizes. The layout hash is FNV-1a 64 over
  `path|type|offset|size` lines. Generated `stc_retain_size/layout_hash/save/load` exist;
  `stc_rt_state_*` are stubs returning 5 (disabled) until M6.

## 7. Build bridge

```text
stc build <file>... [--name <name>] [--emit ir,obj,exe,lib,metadata]
           [--out-dir <dir>] [--target <triple>] [--clang <path>]
           [--runtime <path>] [--relaxed] [--message-format human|json]
```

- Lex/parse/analyze exactly as `stc check`; codegen only if no diagnostics are errors.
- Default `--name` is the sanitized stem of the first sorted input; default emit is `ir`;
  default out-dir is `target/stc/debug`.
- Artifacts: `<name>.ll`, `<name>.o`, `<name>.exe`, `<name>.lib` (MSVC) / `<name>.a` (GNU),
  `<name>.stc-meta.json`.
- Runtime discovery: `--runtime`, `STC_RUNTIME`, beside the `stc` binary, then
  `target/{debug,release}`. Missing runtime is `STC-E-BUILD-003`, exit 2.
- Clang discovery: `--clang`, `STC_CLANG`, `PATH`; `--version` must succeed, else
  `STC-E-BUILD-001`, exit 2. Clang/link failures are `STC-E-BUILD-002`, exit 1, and keep `.ll`.
- Windows MSVC link: discover the MSVC lib directory (vswhere or the standard install roots)
  and the highest Windows SDK `ucrt`/`um` x64 directories, pass `-Xlinker /LIBPATH:...`, and
  link `ws2_32`, `ntdll`, `userenv`, `advapi32`, `bcrypt`, `ole32`, `shell32`, `user32`,
  `dbghelp`. Non-Windows hosts pass the runtime archive directly.
- Shared libraries and exe+lib together are `STC-E-BUILD-004` (exit 2).

## 8. Diagnostic codes

| Code | Meaning |
| --- | --- |
| `STC-E-CODEGEN-001` | valid HIR construct not implemented in M4 |
| `STC-E-CODEGEN-002` | internal codegen failure / IR invariant violation |
| `STC-E-BUILD-001` | clang missing, unsupported, or unusable |
| `STC-E-BUILD-002` | clang invocation or link failed |
| `STC-E-BUILD-003` | runtime staticlib missing or ABI mismatch |
| `STC-E-BUILD-004` | invalid build option/artifact combination |
| `STC-E-EXPORT-001` | invalid/duplicate `{stc:export}`/`{stc:no_mangle}` symbol |
| `STC-E-RUN-001` | multiple tasks/unsupported scheduling (reused) |
| `STC-E-RUN-002` | no runnable program/configuration or ambiguous configuration |

Existing lexer/parser/sema codes are unchanged.

## 9. Test plan

- Scalar mapping, sizes, and canonical type names.
- Mangling/signature-hash stability and overload distinctness.
- Checked arithmetic, division, power, comparisons, casts, control-flow shape.
- Program/global/configuration emission, plan selection, bindings, task guard.
- Process-image layout, defaults, metadata; scalar retain layout/hash.
- Determinism: generate twice, byte equality; no paths/timestamps in output.
- Unsupported M5 constructs produce one `STC-E-CODEGEN-001` with a span.
- 20k-term binary chain lowers without stack overflow.
- Golden `.ll` snapshots using `STC_UPDATE_SNAPSHOTS=1`.
- Clang verification in codegen tests (`STC_REQUIRE_CLANG=1` in CI).
- Driver execution tests: exe with `--cycles N`, arithmetic/control flow/functions/globals,
  traps, cycle persistence, `%M`/`%Q` image access.
- Library tests: static archive + checked-in C harness, exported scalar function, ABI version,
  metadata string, no-op cycle.
- `fuzz_codegen`: parse → analyze → generate on arbitrary input; no panic; bounded output.

## 10. Order of work

1. Decisions D-076–D-093 and this plan.
2. Runtime/link spike (MSVC staticlib, hand-written IR main) — done before codegen.
3. HIR gaps: I/O hints, export names, task folding, LDATE units.
4. `stc-codegen` skeleton: module builder, types/layout, mangling, determinism.
5. Expressions, casts, checked arithmetic, control flow; clang verification.
6. Globals, programs, units, `main`, library exports.
7. `stc build --emit=ir`; then obj/exe/lib and execution/C-harness tests.
8. Process image, metadata, retain ABI.
9. Preflight/errors/JSON/artifact layout/runtime discovery.
10. `fuzz_codegen`, deep-chain, determinism, cross-target smoke.
11. Docs/status, full CI-equivalent checks, commit M4.

## 11. Status (implemented)

- [x] M4 decisions D-076–D-093 recorded; runtime/link spike resolved for Windows MSVC.
- [x] HIR gaps: `{stc:io}` hints on globals/fields, validated `export_name`, folded task
      values, LDATE nanoseconds, configuration fallback rules.
- [x] `stc-codegen`: textual IR builder, scalar layout, canonical names/mangling, checked
      arithmetic, casts, expressions, statements, functions, programs, globals,
      init/cycle/shutdown/main, library surface, export wrappers, process image, scalar retain
      layout, deterministic metadata, preflight diagnostics.
- [x] `stc-runtime`: C ABI, `--cycles N` loop, latched clock, process image, traps (exit 70),
      retained-state stubs, panic boundary.
- [x] `stc build` bridge: `ir`, `obj`, `exe`, static `lib`, `metadata`; clang/runtime discovery;
      Windows MSVC link libraries; JSON diagnostics; exit codes 0/1/2/3; deterministic artifacts.
- [x] Tests: 20 codegen tests, 5 sema gap tests, 12 driver execution/harness tests, IR snapshot,
      clang verification, determinism, deep 20k-term chain, cross-target object emission.
- [x] `fuzz_codegen` target added (60 s CI smoke).

Deviations recorded during implementation:

- The generated library does not define `stc_abi_version`; the runtime owns it, avoiding a
  duplicate symbol (D-086).
- `stc build --emit=lib` still resolves the runtime staticlib (as scoped) but does not embed it
  in the archive; the host links the runtime.
- Input defaults are applied once by `stc_init`, not each cycle, so library hosts can drive
  inputs between cycles (D-084).
- Configuration bindings lower program fields through `This(program)`; the cycle binds that
  pseudo-`this` to the current instance global (D-083).
- `--debug` is accepted in the codegen API but the CLI does not expose debug info yet; M6.
