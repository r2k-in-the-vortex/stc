# Conformance Profile: `stc-2013-oop-utf8-v1`

Status: Target profile; M3 front end, M4 scalar codegen, M4.5 online-change foundation, M5 lowering phases 1–7, `STC-POINTER`, and phase 4b (interfaces, descriptors, `?=`, object references, and their verification) complete (2026-10-25); M6 project system, CLI, scheduler, state, I/O, test runner, formatter, online workflow, Lean vectors, conformance corpus, and CI complete (2026-10-27); M6.5 debug/monitoring pending

Applies to: `stc` v1  
Normative reference: IEC 61131-3:2013 (Ed. 3.0), Structured Text portions  
Project extensions: `STC-UTF8`, `STC-PRAGMA`, `STC-TEST`, `STC-POINTER`  
Runtime capabilities (not IEC claims): online change, GOT/manifest, debug/monitoring

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

Online change, the GOT, the persistent manifest, and debug/monitoring are **project runtime
capabilities**, not IEC language features. They are specified in
[Online Change and GOT](Architecture/Online-Change-and-GOT.md),
[GOT Artifacts and Schemas](Architecture/GOT-Artifacts.md), and
[Debug and Monitoring](Architecture/Debug-and-Monitoring.md). The project is **not a safety PLC
and is not responsible for safety in any way**; debug sessions may hold outputs indefinitely.

**Implementation status (M5):** the lexical surface, the full syntactic surface (CST, typed AST
views, pragma payload validation, recovery), and the semantic surface (collection, namespaces
and `USING`, name resolution, type checking, constant evaluation, initializers, access control,
OOP, references, I/O and retention diagnostics, configurations, and HIR lowering) are
implemented. `stc-codegen` emits deterministic textual LLVM IR for the M5 lowering: scalar and
aggregate values (`STRUCT`, enums, subranges, static and rank-1 conformant arrays,
`STRING`/`WSTRING`/`USTRING`), references and `VAR_IN_OUT`/output parameters, classes and
function blocks with methods/inheritance/`THIS`/`SUPER`, all standard functions, the standard FB
library with cycle-latched timers, and host-rooted retained state for aggregate/string/FB leaves.
Function blocks, OOP lowering, aggregates, strings, references, standard functions, and the
standard FB library are therefore covered by clang acceptance, C-harness execution, and
online-change preservation tests. Phase 4b.1 projects runtime-owned TYPEINFO/VTABLE descriptors,
interface-prototype trap stubs, and hidden `HANDLE`/`OBJREC` roots. Phase 4b.3 lowers object
values as record handles, allocates local class/FB instances and records on the stack, reads and
writes persistent objects through their hidden roots, passes `(handle, table, ...)` to object
bodies, copies class handles versus FB state on assignment, and dispatches every class/FB method
through TYPEINFO and the class VTABLE (`SUPER` calls the base function cell directly). Class
handles that refer to activation-owned records are rejected from escaping. Phase 4b.4 lowers
interface values as 16-byte `{record, per-(object type, interface) vtable}` pairs passed and
returned by value and stored as single roots in fields, globals, program fields, and arrays;
class/FB-to-interface upcasts use the compile-time base-first interface-map index,
interface-to-interface upcasts search the runtime TYPEINFO map, `= NULL`/`<> NULL` compare the
record pointer, and interface calls dispatch through the pair's vtable. Interfaces that refer to
activation-owned records are rejected from escaping. Phase 4b.5 lowers assignment attempts
(`?=`) inline: a class destination checks the object TYPEINFO's base chain, an interface
destination checks membership in the TYPEINFO interface map (including `EXTENDS`), and a
reference destination compares the descriptors; a failed attempt yields the all-zero `NULL`
value and attempts never trap. `REF(object)` produces an `Object` place descriptor whose data is
the record handle and whose descriptor field is the TYPEINFO pointer, so dereference, field
access, whole-object assignment, and object-reference attempts use object semantics;
`REF_TO`-to-interface converts a class/FB object reference and yields a null interface for a
null or incompatible object. Phase 4b.6 verifies the phase with clang acceptance, C-harness
execution, real module warm swaps, and OOP fuzz seeds: preserved interface values and class
handles keep stable TYPEINFO, VTABLE, record, handle, and per-pair vtable addresses, activation
patches the preserved pair to the new method cells, fresh object records and handles initialize
once and roll back, and vtable-shape changes are rejected against an active baseline. Interface
identity comparison and references to split aggregate values remain explicit diagnostics;
pointer-to-object/interface conversions and `ADR` of objects stay rejected. The transient
`STC-POINTER` extension is implemented. M6 is complete (2026-10-27): the `stc.toml` project
manifest and lockfile, package-aware namespaces, `stc run`, `stc test`, file-backed retained
state, I/O device/binding metadata and backends, the formatter, and the online-change workflow
(`rename`, `online download/diff`, `run --watch`) are implemented; M6 also implements cooperative
multi-task scheduling (priorities, intervals, `SINGLE`, task guards, and host-owned run state).
`stc check <file>...` remains the manifest-free single-package bridge. The statuses in §3–§4
describe the v1 target surface. Unsupported constructs are rejected with the diagnostics in
[Grammar](Syntax/Grammar.md) §13 or `STC-E-CODEGEN-001` rather than silently accepted.
The online-change/GOT/manifest/debug architecture is resolved and the M4.5 foundation is
implemented. M4.5 phases 0–10 are complete (2026-10-25):
generated programs execute as ABI 2 loadable modules through the persistent host, with
GOT-rooted state, manifest baselines, a thin launcher, and an embedding host. Scalar hot swaps
preserve compatible roots, initialize new roots once, quarantine removed roots, execute tracked
renames, reject incompatible type/layout changes until a cold rebuild, and reconfigure the
process image/remap direct cells at activation. The phase-9 test/CI pass covers double-build
determinism, malformed manifest/descriptor inputs, concurrent pinning, and the launcher/embedding
artifacts on both hosts. Phase 10 closes the documentation and handover pass: the specification
set and M5 handover describe the implemented host/module model, and the M6.5 interface boundary
is recorded. M5 phases 1–6 extend that foundation to aggregates, references, OOP, the standard
library, and aggregate retained state, including an online change preserving FB, aggregate,
interface, class-handle, and descriptor state; phase 4b.4 implements interface values, upcasts,
and dispatch, phase 4b.5 implements assignment attempts, object references, and
`REF_TO`-to-interface, and phase 4b.6 verifies the phase end to end. Full debug/monitoring is
M6.5.

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
| Unicode identifiers | Not in profile | ASCII-only (A-013); UTF-8 only in comments/strings |
| `//` line comments | Implemented | |
| `(* ... *)` nested block comments | Implemented | |
| `/* ... */` nested block comments | Implemented | |
| Pragmas `{ ... }` | Parsed and attached | `stc:` payloads are strict JSON with duplicate-key/depth/size checks (D-050); others ignored with a warning |
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
| Reference types (`REF_TO`, `REF()`, `^`, `NULL`) | Implemented; persistent-capable values use host-resolved place descriptors |
| `POINTER TO` (`STC-POINTER`) | Project extension: transient-only, full arithmetic/indexing/`SIZEOF`, explicit `REF_TO`↔`POINTER TO` casts. Pointer storage may appear in transient aggregates, never in persistent roots or fields |
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
| Configuration/resource/task/program configuration | Implemented; cooperative multi-task scheduling and host-owned task state (M6) |
| VAR_ACCESS | Implemented; paths/types/direct locations emitted in metadata (M6) |
| Standard functions | Implemented (see §6) |
| Standard function blocks | Implemented as synthesized HIR in `STC_STD` (TON, TOF, TP, CTU, CTD, CTUD, R_TRIG, F_TRIG, SR, RS), not textual `stdlib/` sources (D-134) |
| Ed. 4 properties, mutex, semaphore, ASSERT | Not in profile |

### 4.3 Variable declarations

| Feature | Status |
| --- | --- |
| `VAR`, `VAR_INPUT`, `VAR_OUTPUT`, `VAR_IN_OUT`, `VAR_TEMP` | Implemented |
| `VAR_GLOBAL`, `VAR_EXTERNAL`, `VAR_ACCESS`, `VAR_CONFIG` | Implemented |
| `CONSTANT`, `RETAIN`, `NON_RETAIN` | Implemented; effective policy per leaf; file-backed state in v1 |
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
| `POINTER TO` arithmetic/indexing/`SIZEOF` | Project extension (`STC-POINTER`), transient-only |
| Pointer conversions | Explicit casts `REF_TO T(x)` / `POINTER TO T(x)` only; runtime validation; no pointer↔integer or pointer↔object/interface |
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

These are synthesized as production HIR in the reserved `STC_STD` scope (D-134) and compiled
with the user project. Their timing
behavior uses a single cycle time latched at `stc_module_cycle` entry, so all timers in one scan see the
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
- Multi-threaded ST task scheduling in v1. The host has a cycle thread and, in M6, a
  control/status thread; later milestones may add loader and service threads, but only the cycle
  thread executes ST.
- Human-machine interface and the IEC communication model. Online change and debug/monitoring are
  provided as project runtime capabilities in v1; online change is implemented, while
  debug/monitoring is the M6.5 milestone and is not claimed until it is implemented.
- Safety functions, safety certification, automatic fail-safe outputs, or a safety watchdog. The
  project is not a safety PLC and assumes no safety responsibility.
- Field-layout migration, resizing, and pointer translation in v1. Per-field add/remove/rename
  and tracked moves are supported; incompatible changes reset or require a cold download.
- Remote/network deployment, multi-user scheduling, and remote debugging in v1.
- User-visible `__NEW`/`__DELETE` in v1; a separate v2 vendor extension.
- A network package registry and version-solving resolver in v1. Local path dependencies are in
  v1; a registry/version solver is a later extension (A-009).
- Hardware I/O backends and schedules (EtherCAT, Modbus, MQTT) in v1. They are v2; v1 uses
  null/memory/file backends and runtime-owned binding.
- `PERSISTENT` (not a 2013 keyword) and periodic/checkpointed state saves. v1 saves retained
  state at normal shutdown.

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
| Time source | One cycle time latched at `stc_module_cycle` entry; all timers in a scan see the same value |
| BOOL representation | `i8` in memory, `i1` in SSA; normalized on conversion |
| STRING representation | Length-prefixed fixed-capacity buffer; see runtime spec |
| USTRING representation | Length in characters + fixed `[n x i32]` scalar buffer; 4 bytes/character; static |
| USTRING assignment | Capacity is part of the type; cross-capacity assignment/argument passing inserts a runtime check and traps on overflow |
| Array bounds | Checked by default in hosted builds; release policy documented |
| I/O binding | Runtime-owned; compiler emits process-image/device/binding metadata; `{stc:io}` JSON hints; runtime may override |
| Unbound I/O | Inputs receive type defaults (or declared initializers) each cycle; outputs are written but discarded at cycle end; `%M` persists |
| Retained state | File-backed, versioned, layout-hashed, CRC-checked; warm restart restores, cold start initializes; `on-mismatch` is `error` or `reset`; atomic saves at shutdown |
| Assertions | `STC_ASSERT` is test-only; test-only units are excluded from production builds; production use is an error |
| Variable-length arrays | Conformant parameters only; descriptor points at caller storage; no dynamic allocation |
| Subrange assignment | Checked |
| Identifier alphabet | ASCII letters/digits/underscore in v1 |
| Identifier length | Implementation limit 1024 bytes; at least 255 accepted (STC-LEX-016) |
| Invalid UTF-8 recovery | One U+FFFD per invalid sequence; lexing continues; diagnostics keep the original byte offset and bytes (STC-LEX-002) |
| BOM handling | An initial UTF-8 BOM is trivia and does not shift the first column; a BOM elsewhere outside comments, strings, and pragmas is an error (STC-LEX-001, STC-LEX-003) |
| Trivia attachment | Same-line horizontal whitespace and single-line comments trail the previous token; line breaks and multi-line comments lead the next token |
| UTF-8 string escapes | `$HH`, `$HHHH`, and `${HEX}` accepted; surrogates and values above U+10FFFF rejected lexically (STC-LEX-034, STC-LEX-036) |
| Typed string/char literals | Typed prefixes determine encoding and accept either quote delimiter; unquoted `UCHAR#` accepts one UTF-8 escape (D-044) |
| Default STRING/WSTRING/USTRING capacities | STRING/WSTRING 255 units; USTRING 255 characters (1020 bytes of data) unless declared |
| Case normalization | Uppercase ASCII for symbol keys; original spelling retained |
| Comparison precedence | Annex A split: relational (`< > <= >=`) binds tighter than equality (`= <>`); both left-associative |
| Semicolon handling | Strict by default: every statement terminates with `;`; `--relaxed` allows omitted semicolons after block statements |
| Parser nesting limit | 256 nested expression, statement, and type levels; exceeding it is `STC-E-GRAM-109` (D-047) |
| Parser recovery | Missing tokens are zero-width leaves with a diagnostic; a mismatched `END_*` reports both spans and is left for the enclosing construct (D-053) |
| Statement classification | `x := y` is an assignment; a dereference on the left or `REF(...)`/`NULL` on the right is a reference assignment, finalized by the analyzer (D-055) |
| Pragma duplicates | Duplicate `stc:io` is `STC-E-IO-003`; `stc:io` without `AT` is `STC-W-IO-002` (D-049) |
| Task scheduling | Host-owned cooperative scheduler in M6; tasks share one scan, one latched clock, one input latch, and one output publish. Lower numeric priority is higher; an absent priority sorts after explicit priorities; ties follow resource/task declaration order. No interval = every scan; interval = first run after one interval, then `now + interval` with no catch-up; `SINGLE` runs once and completes; a task/resource topology change requires a cold download |
| Project model | `stc.toml` packages with local path dependencies, deterministic topological order, a committed `stc.lock`, and `--locked`; package identity is `[package].name`, an alias sets only the root namespace, and `INTERNAL` is package-private |
| Test runner | One cold test artifact per test with a synthetic single-task configuration; process-isolated and sequential; default 5 s timeout; memory I/O unless `[test].io` overrides; mock clock from `[test].mock-clock` |
| Formatter | Fixed style over the lossless CST; comments, blank lines, and existing line breaks preserved; per-file BOM and line-ending encoding preserved; strict semicolons inserted; `--check`; idempotent |
| Online workflow | Active-baseline builds, `stc online download/diff`, committed `stc.renames.toml` tracked renames, and `stc run --watch` over a local user-only deploy/status endpoint with a per-run nonce |
| Execution artifacts | `module` is a loadable shared library paired with a GOT manifest; `exe` is a thin host launcher; `lib` embeds the host; host ABI 2.2 implemented (test-clock service ID 12; production modules require 2.1); M6.5 appends debug service ID 13 at minor 2.3 (implementation pending) |
| Persistent memory | Host-owned GOT cells and stable roots; per-field recursive roots; handle-based objects; runtime-owned descriptors |
| References and pointers | Persistent-capable `REF_TO` uses place descriptors; pointer→`REF_TO` is runtime-validated and yields `NULL` on failure; `POINTER TO` is transient-only (`STC-POINTER`) |
| Online change | Whole-module load/prepare, O(1) cycle-boundary activation, one-successful-cycle rollback, off-thread unload |
| Baseline policy | Exact parent build ID + digest pairing + profile/target/ABI/layout match; missing/stale/invalid baseline forces a cold download |
| Preservation | Matching `GotId`/type/layout/size roots preserved; new roots initialized; removed roots quarantined; no field-layout migration in v1 |
| Debug builds | Debug is the default build form. M6.5 emits DWARF (ELF) or CodeView (MSVC) with deterministic paths/order and no timestamps; `--release` selects optimised, stripped, hook-free artifacts with distinct build IDs/manifests and default out-dir `target/stc/release`; debug stays `target/stc/debug` (D-178) |
| Debug behavior | Typed read/write/force/watch, checkpoint breakpoints, stop-the-world stepping, call stacks with optimized-out reporting; writes/forces apply only at cycle boundaries; swaps and the pause bound end sessions and clear state; outputs may be held indefinitely; not a safety PLC (D-173–D-177) |
| Debug protocol | Local-only `stc-debug/1` on a user-scoped pipe (default) or loopback socket (optional); newline-delimited JSON, per-run token, four scopes, audited protected access, rate limits, and bounded frames; separate from the M6 control channel (D-173, D-174) |
| Debug checkpoints | Dense checkpoint table in the metadata sidecar, source-span bound, first-class host check service ID 13 at ABI 2.3; debug builds set `entry_flags` bit 4/bit 6 and `required_features` bit 4 when hooks are emitted; release and test builds are hook-free (D-177) |
| Debug sessions | One session per host; start does not stop the cycle; a stop parks the cycle at a checkpoint and holds outputs; a five-minute default stop/idle bound, swap/disconnect/timeout teardown, force/breakpoint clearing, and explicit reconnect with no state migration (D-175) |
| Runtime threads | One cycle thread and, in M6, one control/status thread for deployment/status; M6.5 adds service/monitor threads. Only the cycle thread executes ST; generation pinning for readers |

## 9. Project model (project feature, not an IEC claim)

- A package is an `stc.toml` manifest plus sources. Dependencies are local path packages in v1,
  pinned by a committed `stc.lock` with `--locked` enforcement.
- The dependency graph must be acyclic; packages compile topologically; duplicate canonical paths
  are deduplicated.
- Package-aware namespaces: a package's identity is its `[package].name`; a dependency alias sets
  only the root namespace. Catalog keys, `GotId` scopes, and mangled symbols use the package
  name, so an alias rename never changes identity. `INTERNAL` is package-private and compares
  package identity; `PRIVATE` is POU/class-private; top-level declarations are public across
  packages by default.
- A separate test project is a normal package that depends on the code under test; `stc test`
  runs only the current package's test POUs.
- No network access, registry, or version solving in v1; the lockfile pins the local path graph
  (A-009).

## 10. Diagnostic families and codes

Existing lexical, grammatical, semantic, codegen, I/O, retain, module, manifest, ABI, and
online-change families stay in their owning specifications; this section indexes the codes added
or fixed for the project system, M6, and M6.5. A code has one meaning; a code is never reused.
The owning specification is normative if a summary here disagrees.

| Code | Condition | Owner |
| --- | --- | --- |
| `STC-E-PROJECT-001` | `stc.toml` cannot be read or parsed | CLI and Manifest §2 |
| `STC-E-PROJECT-002` | schema/type/unknown-key/invalid-value error; snake_case spellings are unknown keys | CLI and Manifest §2 |
| `STC-E-PROJECT-003` | package name/version/language missing or invalid | CLI and Manifest §2 |
| `STC-E-PROJECT-004` | source glob/path invalid, outside the project, ambiguous, or matching nothing | CLI and Manifest §2.3 |
| `STC-E-PROJECT-005` | dependency path/alias/name invalid, missing `path`, or alias/name mismatch | CLI and Manifest §2.2 |
| `STC-E-PROJECT-006` | dependency cycle or duplicate canonical package path | CLI and Manifest §2.2 |
| `STC-E-PROJECT-007` | configuration selection missing or ambiguous | CLI and Manifest §2.2 |
| `STC-E-PROJECT-008` | `{stc:test}`/`[[test.case]]` conflict or invalid test metadata | CLI and Manifest §2.4 |
| `STC-E-PROJECT-009` | I/O device/channel/binding invalid | CLI and Manifest §2.2; I/O §3 |
| `STC-E-PROJECT-010` | build/run/retain/online/debug setting invalid | CLI and Manifest §2.2 |
| `STC-E-PROJECT-011` | lint configuration invalid, unknown code, or downgrades an error | CLI and Manifest §2.2 |
| `STC-E-PROJECT-012` | `stc.renames.toml` cannot be read/parsed or has an invalid schema | CLI and Manifest §3.1 |
| `STC-E-PROJECT-013` | rename history conflict, duplicate mapping, or cycle | CLI and Manifest §3.1 |
| `STC-E-LOCK-001` | lockfile missing under `--locked` | CLI and Manifest §3 |
| `STC-E-LOCK-002` | lockfile out of date | CLI and Manifest §3 |
| `STC-E-LOCK-003` | lockfile malformed or unsupported schema | CLI and Manifest §3 |
| `STC-E-ASSERT-001` | production POU uses `STC_ASSERT` | Testing and Verification §2.2 |
| `STC-E-ASSERT-002` | `STC_ASSERT` used outside a test-only unit or with an invalid form | Testing and Verification §2.2 |
| `STC-E-TEST-001` | production POU references a test-only POU | Testing and Verification §2.2 |
| `STC-E-TEST-002` | duplicate test name or duplicate `[[test.case]]` entry | Testing and Verification §2.1 |
| `STC-E-TEST-003` | invalid test signature (function inputs/return, FB/program shape, `PASSED` type) | Testing and Verification §2.1 |
| `STC-E-TEST-004` | test-source conflict or invalid test-only reference | Testing and Verification §2.1 |
| `STC-E-FMT-001` | syntax errors prevent formatting | CLI and Manifest §1.5 |
| `STC-E-FMT-002` | file cannot be read or written | CLI and Manifest §1.5 |
| `STC-E-ONLINE-007` | control channel unavailable or unauthorized | Online Change and GOT §10 |
| `STC-E-ONLINE-008` | deploy rejected (incompatible candidate or prepare failure) | Online Change and GOT §8 |
| `STC-E-ONLINE-009` | active manifest unavailable for download/diff | Online Change and GOT §7.4 |
| `STC-E-RUN-003` | invalid `stc run` option or runtime scheduling configuration (for example a task without a guard); codegen configuration errors stay `STC-E-RUN-001/002` | Runtime and Execution §1.3 |
| `STC-E-RUN-IO-001` | the file I/O backend cannot write its output image at shutdown | I/O and Devices §7 |
| `STC-W-IO-004` | the file I/O backend input image is missing or unreadable; the run starts with defaults | I/O and Devices §7 |
| `STC-W-PROJECT-001` | setting parsed but not honored until a later milestone (for example a configuration key reserved ahead of its milestone) | CLI and Manifest §2.2 |
| `STC-W-PROJECT-002` | rename history has no baseline; the new key receives a fresh `GotId` | CLI and Manifest §3.1 |
| `STC-W-ONLINE-007` | compatibility-report warning not covered by an existing online code | CLI and Manifest §1.6 |

`STC-W-ONLINE-002` remains the retained-initializer warning used by `stc-got`; it is not reused
for compatibility reports. Existing `STC-E-RUN-RETAIN-*`, `STC-E-ONLINE-STATE-*`, and
`STC-W-ONLINE-STATE-*` codes are unchanged.

### 10.1 Debug diagnostics (M6.5)

The `DEBUG` family is reserved for the local debug protocol, session, typed access, checkpoint,
and build rules of [Debug and Monitoring](Architecture/Debug-and-Monitoring.md) §13–§18. The
codes are stable; the phase-0 design fixes them before implementation. Initial errors and
warnings are `not-implemented` in the traceability allowlist until phases 3–8 add tests.

| Code | Condition | Owner |
| --- | --- | --- |
| `STC-E-DEBUG-001` | debug service unavailable or disabled: no capable module/server, or policy disabled it | Debug and Monitoring §13.3, §18.1 |
| `STC-E-DEBUG-002` | unsupported protocol version or wrong protocol/endpoint (including a control endpoint) | Debug and Monitoring §13.2, §18.1 |
| `STC-E-DEBUG-003` | malformed/invalid request: unknown key/op/field/scope/kind, bad parameter, invalid request ID, raw-address field | Debug and Monitoring §13.2–§13.4 |
| `STC-E-DEBUG-004` | oversized or unterminated frame; connection closes | Debug and Monitoring §13.2, §13.6 |
| `STC-E-DEBUG-005` | authentication failed: missing/malformed/wrong token; connection closes and failures are limited | Debug and Monitoring §13.3 |
| `STC-E-DEBUG-006` | authorization denied: scope not granted or write/force/protected read denied by policy | Debug and Monitoring §13.3 |
| `STC-E-DEBUG-007` | invalid session state: no session, session already active, or operation before start | Debug and Monitoring §14.1, §14.2 |
| `STC-E-DEBUG-008` | unknown target/path/`GotId`/frame/local/checkpoint/breakpoint/watch/stop ID | Debug and Monitoring §15.1, §15.8 |
| `STC-E-DEBUG-009` | invalid target or value: type/shape/writability/range/enum/code point/capacity | Debug and Monitoring §15.5, §15.8 |
| `STC-E-DEBUG-010` | stale generation/layout: expected generation/build/type/layout differs, removed target, or change during read | Debug and Monitoring §15.1–§15.3 |
| `STC-E-DEBUG-011` | limit exceeded: queue, breakpoints, watches, aggregate leaves, response size, or backpressure | Debug and Monitoring §13.5 |
| `STC-E-DEBUG-012` | request or watch rate limit exceeded | Debug and Monitoring §13.5 |
| `STC-E-DEBUG-013` | request processing timeout; retry/reconciliation rules apply | Debug and Monitoring §13.5, §13.6 |
| `STC-E-DEBUG-014` | breakpoint/step/pause unavailable: no hooks or no frame/checkpoint information | Debug and Monitoring §16.6, §16.8 |
| `STC-E-DEBUG-015` | invalid step/continue, stale stop frame, or unsatisfiable step | Debug and Monitoring §14.4, §16.8 |
| `STC-E-DEBUG-016` | invalid debug metadata: table/span/function/build-ID/flags/size or hook agreement | Debug and Monitoring §16.2, §16.7 |
| `STC-E-DEBUG-017` | source/locals/stack debug information unavailable (stripped/optimized out) | Debug and Monitoring §15.2, §16.6 |
| `STC-E-DEBUG-018` | stop or idle session bound exceeded; session ends, state clears, cycle resumes | Debug and Monitoring §14.3 |
| `STC-E-DEBUG-019` | session ended by an online swap; reconnect against the new generation | Debug and Monitoring §14.5 |
| `STC-E-DEBUG-020` | transport/endpoint failure: bind/listen/path/permission/symlink/unsupported transport | Debug and Monitoring §13.1 |
| `STC-E-DEBUG-021` | audit unavailable: log cannot be created, secured, or appended; fail closed | Debug and Monitoring §13.3 |
| `STC-E-DEBUG-022` | internal debug service error; session ends if state is untrusted | Debug and Monitoring §13.6, §18.1 |
| `STC-W-DEBUG-001` | breakpoint unbound; reported, not moved | Debug and Monitoring §16.7 |
| `STC-W-DEBUG-002` | frame/local optimized out or unavailable; not reconstructed | Debug and Monitoring §15.2, §16.8 |
| `STC-W-DEBUG-003` | watch batch coalesced/dropped under rate/backpressure | Debug and Monitoring §15.7 |
| `STC-W-DEBUG-004` | debug information or hooks unavailable; monitoring-only session | Debug and Monitoring §16.6 |
| `STC-W-DEBUG-005` | forced retained value may be saved by normal shutdown; force table is not persisted | Debug and Monitoring §4, §14.7 |

## 11. Claim format

Any compliance statement produced by the project shall state:

- profile ID and version,
- the exact supported feature list (this document),
- the project language extensions (`STC-UTF8`, `STC-PRAGMA`, `STC-TEST`, `STC-POINTER`),
- the runtime capabilities (online change, GOT/manifest, debug/monitoring) separately from the
  IEC language claim, including the debug protocol version (`stc-debug/1`), build mode
  (`debug`/`release`), debug-information format (`dwarf`/`codeview`/`none`), and checkpoint-hook
  capability,
- the implementation-defined choices in §8,
- that Ed. 4 conformance is not claimed,
- that the project is not a safety PLC and assumes no safety responsibility.

The compiler should be able to emit this statement (or a machine-readable equivalent) from
`stc --compliance`.
