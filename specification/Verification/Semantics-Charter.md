# Semantics Charter — Phase 0 Core

Profile: `stc-2013-oop-utf8-v1`
Date: 2026-10-27
Status: Approved 2026-10-27. P0.0 froze the scope and the claim. P0.1 and P0.2 are complete, P0.3
is paused mid-proof with its drift register rechecked on 2026-10-31, and P0.4 is in progress
(WL-3). The frozen target scope was revised on 2026-10-31 to remove `switch`; no other inclusion
or exclusion changed.
Parent: [Verification Program Plan](Plan.md)
Phase plan: [Phase 0 Implementation Plan](Plan-Phase0.md)
Semantics reference: [stc Semantics — Phase 0 Core](Semantics.md)

## 1. Purpose

This charter fixes what the Phase 0 semantics is for, what it claims, what it trusts, and how it
may change. It is the agreement that the definitions and proofs in `Stc.Syntax`, `Stc.Grammar`,
`Stc.Hir`, `Stc.Sem`, `Stc.Codegen`, and `Verification.Semantics` are written against. Later
phases cite this charter and the [semantics document](Semantics.md) by version; they do not
redefine the core informally.

The charter exists so that a parser, type-checker, or codegen proof is aimed at a reviewed
meaning rather than at an accidental property of the current Rust implementation.

## 2. Claim and non-claims

The Phase 0 claim, at level S in the program's claim ladder, is:

> The stc core language subset and the emitted-IR fragment it compiles to are defined by a
> machine-checked semantics. The core source semantics is deterministic, type-sound, and
> trap-sound; the core checker accepts only well-typed programs; the emitted-IR semantics is
> deterministic and memory-, GOT-, and trap-sound on the covered fragment; and the executable
> reference models are equivalent to the declarative judgments. The Rust compiler is compared
> with the reference semantics on the core corpus as V0 evidence.

Phase 0 does **not** claim:

- that the Rust parser, analyzer, or code generator refines the semantics (Phase 1 and later);
- correctness of the textual IR printer or parser (Phase 5);
- a semantics for general LLVM, its optimizer, its backend, or its memory model;
- coverage of the full language: floating point, strings, aggregates, OOP, references,
  function blocks, tasks, I/O, retained state, online change, and debug are excluded by §6;
- safety certification, freedom from hazards, or suitability for a safety function.

The conformance harness is differential evidence. Agreement on the corpus is not a refinement
proof, and a disagreement is not by itself a compiler bug.

## 3. Trusted computing base

The semantics is checked relative to the following trusted base:

- the Lean 4.35.0 kernel, its standard library, and pinned toolchain in
  `lean/lean-toolchain`;
- the definitions in this repository: theorems are true of the Lean definitions, so a wrong
  definition is a wrong semantics even when the proof is checked;
- Batteries for executable models where needed. Mathlib is permitted only in proof-only
  libraries after a build-time spike (P0-D5); it never enters the executable interpreter, the
  dump reader, or the conformance harness;
- the HIR dump, the case descriptors, and the report schema as inputs to the harness. They are
  versioned and validated, but their content is trusted to describe the compiled program;
- the Rust compiler, clang/LLVM, the operating system, and the hardware as the *oracle* for
  conformance evidence. Bugs in that stack appear as disagreements or as unmodeled features;
  they are not ruled out by the semantics.

Trusted-theorem policy: no `sorry` or `admit` in proved modules; top-level theorems are audited
with `#print axioms`; the only permitted axioms are Lean's `propext`, `Classical.choice`, and
`Quot.sound`. `native_decide` is not used in a trusted theorem unless the extra trust is
recorded in the registry and accepted in review. Every exception is recorded in
[`lean/verification-status.toml`](../../lean/verification-status.toml) and in the semantics
document's TCB section.

## 4. Relationship to the program plan

[Phase 0](Plan.md#phase-0--formal-semantics-foundation-and-soundness) is standalone: it uses no
Aeneas/Charon translation and requires no Rust refinement. Its outputs are consumed as follows.

| Consumer | Consumes |
| --- | --- |
| Phase 1 | The recursive models, loop-shaped step relations, and their equivalence theorems as refinement targets; the registry format; the no-`sorry` and axiom checks |
| Phase 2 | The core source semantics and conversion relation for constant-evaluation refinement |
| Phase 4 | The surface AST, `Derives`, and the AST/CST and source-span mappings for parser contracts |
| Phase 5 | `IRFrag` and `WellFormed` as the starting point for the full emitted-IR fragment, and the proved structural lemmas |
| Phase 6 | The core typing judgment and dynamic semantics as the extension point for the full language |
| Phase 7 | The source and target semantics for the codegen preservation theorem |
| Phase 8 | The GOT cell and trap models for runtime and swap invariants |

Phase 0 adds no claim above S for the semantics and no claim above V0 for the Rust compiler.
Later phases extend the core; they do not silently change it (§5).

## 5. Versioning and amendment

The semantics is a product artifact.

- `semantics_version` is `1.0-draft` while Phase 0 is in progress. It becomes `1.0` at the P0.7
  freeze, and the registry records the version and the freeze commit.
- The HIR dump carries its own format version. Every conformance report records both the
  semantics version and the dump-format version.
- A new feature is added in new modules or new sections and leaves existing definitions and
  theorem statements intact.
- A change to an existing definition or theorem is a semantics change: it requires a version
  bump, a registry update, a changelog entry in [Semantics.md](Semantics.md), and a note in
  [Plan.md](Plan.md) if it changes a phase claim.
- A change to the frozen core scope in §6 also requires this charter to be revised and the
  parent program plan to be updated. Extending the core for a later phase is expected; silently
  shrinking or reinterpreting it is not.
- The registry is the single source of truth for theorem names, specification references, and
  status. The semantics document's theorem section is checked against it.

## 6. Frozen core and exclusion register

The normative definition of the frozen core is [Plan-Phase0.md](Plan-Phase0.md) §3.3 (source
subset) and §3.4 (target fragment). This charter records the decision and every exclusion with
the phase that will add it.

Included in Phase 0:

- source: `BOOL`, signed and unsigned integers, bit strings, and character types; scalar `VAR`,
  function `VAR_INPUT`, scalar `VAR_GLOBAL` roots, `FUNCTION`, and `PROGRAM`; literals, variable
  access, exact operators, comparisons, integer/bit/char conversions, `SEL`, `MIN`, `MAX`,
  `LIMIT`, `MUX`, and core calls; assignment, `IF`, `CASE`, `FOR`, `WHILE`, `REPEAT`, `EXIT`,
  `CONTINUE`, `RETURN`; one program and one sequential cycle; exact standard functions;
  `OVERFLOW`, `DIV_ZERO`, and `RANGE` traps;
- target: `void`, `i1`, `i8`, `i16`, `i32`, `i64`, and `ptr`; the `{ iN, i1 }` overflow result and
  fixed `[N x ptr]`/`[N x i8]` arrays; integer and bitwise instructions including the rotate
  intrinsics, comparisons, casts, `select`, `phi`, `br`, `ret`, `unreachable`, direct and indirect
  `call`, `alloca`, `load`, `store`, stack-slot/root-array/slot-table `getelementptr`, checked
  arithmetic intrinsics with `extractvalue`, and trap calls; the slot table with five cell kinds
  (`Data`, `Function`, `Descriptor`, `Direct`, `Import`), the function-address table, and
  module-local constants; host ABI 2 entry points (production modules require minor 2.1); stack
  frames and GOT cells with no heap.

| Excluded from Phase 0 | Examples | Added by |
| --- | --- | --- |
| Floating-point source semantics | `REAL`, `LREAL`, floating-point operators and standard functions | Phase 6; exact kernels remain integer-only |
| Time and date source semantics | `TIME`, `DATE`, `TOD`, `DT` and their operations | Phase 6 |
| Strings | `STRING`, `WSTRING`, `USTRING`, string functions and conversions | Phase 6 source; Phase 5/7 IR; Phase 8 retained state |
| Aggregates | Arrays, structs, enums, subranges, aggregate initializers | Phase 6 source; Phase 5/7 IR; Phase 8 retained state |
| Object orientation and blocks | Classes, function blocks, interfaces, methods, `THIS`, `SUPER`, namespaces, configurations, resources | Phase 6 source; Phase 5/7 IR descriptors; Phase 8 runtime |
| References and parameter modes | References, pointers, `VAR_OUTPUT`, `VAR_IN_OUT`, aliasing | Phase 6 source; Phase 5/7 IR |
| Tasks and scheduling | Multiple tasks, task configuration, cycle scheduling policy | Phase 6 source; Phase 8 runtime |
| Standard function blocks | Timers, counters, edge detection, and the rest of the standard FB library | Phase 6 source; Phase 8 runtime |
| I/O, retained state, and online change | `AT` bindings, `RETAIN`, persistent image, swap, rollback, unload | Phase 6 source; Phase 8 runtime |
| Additional traps | Bounds, null reference, conversion-to-real, and traps that depend on excluded types | Phase 6 source; Phase 7 target |
| Target floating point and general values | `float`, `double`, vectors, `fadd`/`fsub`/`fmul`/`fdiv`/`frem` | Phase 5 syntax; Phase 7 semantics |
| Advanced target instructions | Atomics, volatile access, `invoke`/`landingpad`, dynamic `alloca`, `insertvalue`, `llvm.memcpy`, `ptrtoint`/`inttoptr`, descriptor/vtable/TYPEINFO blobs, process image, string data | Phase 5 syntax; Phase 7 semantics; Phase 8 runtime |
| Dense-label `switch` lowering | `CASE` currently lowers to comparison branches; no `switch` is emitted | Phase 5 syntax if lowering changes |
| Debug and monitoring | DWARF/CodeView contents, `stc-debug/1`, sessions, typed access, forcing, checkpoint hooks and metadata, host service 13 | Not planned for v1 verification: the verified release build excludes debug state by contract (D-178); see [Plan](Plan.md) §11 |
| Full host lifecycle | Attach/prepare/activate/commit/rollback/unload beyond the initialization boundary | Phase 8 |
| General LLVM model | Poison/`undef` generality, provenance, optimizer behaviour, general pointer arithmetic | Not planned; a non-goal of the program |
| Printer and parser proof | Textual IR printing and parsing round-trip | Phase 5 |
| Compiler correctness | Source-to-IR preservation for the core | Phase 7 |
| Safety certification | Any safety-integrity claim | Not planned; a non-goal of the program |

A feature moves from this table only through a charter revision and a matching parent-plan
update. The source and target semantics modules stay free of excluded features so that adding
one later is an extension, not a rewrite.

## 7. Phase 0 decisions

The ten Phase 0 decisions are resolved and frozen; [Plan-Phase0.md](Plan-Phase0.md) §14 is the
normative record, and this table confirms them for the charter.

| ID | Subject | Confirmed resolution |
| --- | --- | --- |
| P0-D1 | Exact core language subset | Use Plan-Phase0 §3.3 as written; exclude floats, strings, aggregates, OOP, references, FBs, tasks, I/O, and online change |
| P0-D2 | Target IR fragment and memory model | Use Plan-Phase0 §3.4 as written; abstract GOT cells and stack frames; exclude general LLVM memory and UB |
| P0-D3 | Small-step versus functional semantics | Define both the recursive executable model and the loop-shaped small-step relation; prove them equivalent and expose both as refinement targets |
| P0-D4 | HIR dump format and version | Stable, versioned JSON with schema validation; no Rust internals; start from the `stc-hir` dump |
| P0-D5 | Mathlib/Batteries policy | Core Lean plus Batteries for executable models; Mathlib proof-only after a build-time spike |
| P0-D6 | Floats, time, strings, and aggregates | Excluded from Phase 0 by P0-D1; later phases own them |
| P0-D7 | Conformance oracle | Compare the compiled artifact for execution and `stc test` for function cases; keep a no-clang pure-function path |
| P0-D8 | Semantics document location | `specification/Verification/Semantics.md`, versioned with the plan |
| P0-D9 | Recursion and fuel | Include recursion, fuel the interpreter, and do not require a source termination proof |
| P0-D10 | Conformance acceptance threshold | Every core case agrees or has a written triage exclusion; no silent exclusions |

## 8. P0.0 exit record

P0.0 creates the scope freeze, the module skeleton, the registry, the CI checks, and the
semantics document skeleton.

| Deliverable | Location | State |
| --- | --- | --- |
| This charter and the frozen scope | `specification/Verification/Semantics-Charter.md` | Approved 2026-10-27 |
| Semantics document skeleton | [Semantics.md](Semantics.md) | Approved 2026-10-27 |
| Lean module skeletons | `lean/Stc/{Syntax,Grammar,Hir,Sem,Codegen}` and `lean/Verification/Semantics` | Builds with `lake build` |
| Theorem registry | [`lean/verification-status.toml`](../../lean/verification-status.toml) | 24 inventory records, all `planned` |
| Registry check | `tools/ci/check_verification_registry.py` | Schema, section, module, resolution, and axiom checks |
| Conformance placeholder | `lake exe semantics-conformance` | Fail-closed: exits 3 while the core corpus is empty |
| Placeholder check | `tools/ci/check_semantics_placeholder.py` | Requires the placeholder to fail closed; requires real runs once cases exist |
| No-`sorry` and axiom checks | `.github/workflows/ci.yml`, `lean` job | Active for Phase 0 modules |
| Documentation checks | `tools/ci/check_docs.py` | Requires the charter, phase plan, and semantics document |

Review approved this charter, the exclusion register, and the [Semantics.md](Semantics.md)
skeleton on 2026-10-27. The approval is recorded in [Plan-Phase0.md](Plan-Phase0.md) and will be
carried into `Phase0-Report.md` at P0.7.
