# Core Semantics Cases and Reference Results

Profile: `stc-2013-oop-utf8-v1`
Date: 2026-11-02
Semantics version: `1.0-draft`
Case format: `stc-core-case` version `0.1.0`
Result format: `stc-sem-result` version `0.1.0`
Status: P0.6 complete (approved 2026-11-02). The corpus under `tests/verification/semantics/` holds 74 cases;
the frozen dump, the reference interpreter, and the compiled artifact agree on every case. The
coverage inventory, justified exclusions, and triage log are in §7. The byte-stable report is
committed as `tests/verification/semantics/conformance-report.json`, and CI regenerates it and
rejects a stale copy (§8.2). This is V0 differential evidence, not a refinement proof.
Companion: [Core HIR Dump](HIR-Dump.md) · [Semantics](Semantics.md) ·
[Phase 0 Implementation Plan](Plan-Phase0.md)

This document defines the two machine-readable artifacts of the reference-interpreter path: the
case descriptor read by `lake exe stc-sem` and the result object it writes. It is **V0
differential evidence, not a refinement proof**, like the dump itself. The corpus layout is in
§7; the formats are frozen at the versions above.

## 1. Purpose and non-goals

A case descriptor names one entry point of a core-HIR unit, the requested cycle count, the fuel
bound of the reference interpreter, the core features the case covers, and the expected
observable result. A result records what the reference machine actually did: the terminal
status, the terminal value or trap, the global snapshot, the completed cycles, and an optional
per-step trace. The conformance harness compares the two.

The formats are not:

- a source-language test format: the case names a program in a `stc-core-hir` dump, never a
  source file;
- a specification of the Rust compiler's artifact or exit codes: those belong to the harness;
- a proof-carrying format: expected values are test data, not theorems.

`stc-sem` never changes its observation to agree with `expected`; it reports the reference
result and the harness decides whether it matches. A reference result of `outOfFuel` or `stuck`
is always a failure for a case whose expectation is `halted` or `trap`.

## 2. Encoding and determinism

Each file is exactly one UTF-8 JSON object followed by a newline. JSON object members are read by
name; member order is not significant. All numbers are JSON integers; no floating-point value is
accepted or emitted. A producer may pretty-print a case file because only the parsed object
matters. `stc-sem` writes its result with Lean's canonical JSON compressor, so the same input
produces the same result bytes on the same toolchain.

The result carries the case name, the entry point, the semantics version, the dump-format
version, and the case-format version so a report can attribute every comparison to exact inputs.
It does not carry a timestamp, host name, or path, so a produced result is reproducible.

## 3. Case descriptor `stc-core-case` `0.1.0`

A corpus descriptor lives at `tests/verification/semantics/<case>/case.json`; the enclosing
directory name is normally the case name, but the `0.1.0` reader does not enforce that identity.
The top-level object has exactly these members:

| Field | Type | Meaning |
| --- | --- | --- |
| `format` | string | Exactly `stc-core-case` |
| `formatVersion` | string | Exactly `0.1.0` |
| `semanticsVersion` | string | Exactly `1.0-draft` |
| `profile` | string | Exactly `stc-2013-oop-utf8-v1` |
| `name` | string | Case name, `[A-Za-z0-9][A-Za-z0-9._-]*` |
| `entry` | object | Entry point, §3.1 |
| `inputs` | array | Required; must be empty in `0.1.0` |
| `cycles` | non-negative integer | Requested program cycles |
| `fuel` | positive integer | Reference-interpreter fuel bound |
| `features` | array of strings | Coverage tags, §3.3 |
| `expected` | object | Expected observation, §3.2 |

The version, semantics version, and profile are exact: a reader that understands only `0.1.0`
rejects every other value, and it never applies best-effort interpretation. Members not defined
above are not part of the contract; a producer must not rely on them, and a reader ignores them.

### 3.1 Entry object

| Field | Type | Meaning |
| --- | --- | --- |
| `kind` | string | Exactly `program`; the core subset has no other entry |
| `name` | string | Canonical program name in the dump |

The entry name uses the canonical uppercase dot-joined grammar of
[HIR-Dump.md](HIR-Dump.md#2-encoding-identity-and-determinism). The reader rejects a kind other
than `program`, an empty or non-canonical name, and an entry name that does not match the dump's
program. `inputs` exists so a later version can pass arguments without changing the object
shape; a non-empty array is rejected rather than silently ignored because the only `0.1.0` entry
kind is a parameterless program.

### 3.2 Expected object

| Field | Type | Meaning |
| --- | --- | --- |
| `status` | string | `halted` or `trap` |
| `trap` | string or null | Trap name for `status = "trap"`; null for `halted` |
| `globals` | array of integers | Expected global values in unit declaration order |
| `cycles` | non-negative integer | Expected completed cycles |

Trap names are exactly `OVERFLOW`, `DIV_ZERO`, and `RANGE`. A `halted` case must have a null
trap; a `trap` case must name one. `globals` is positional: it must have one value per global in
`unit.globals`, in that order, and the run rejects a count mismatch before stepping. Types are
not repeated because the dump declares them; a result repeats each name and type only so the
snapshot is readable on its own.

`outOfFuel` and `stuck` are reference observations, never expected outcomes. A case cannot ask
for them, and a result that reports one fails comparison.

### 3.3 Coverage feature tags

Feature tags are lowercase `[a-z0-9][a-z0-9._-]*` strings, strictly sorted in byte order and
duplicate-free. They describe the core constructs the case is meant to exercise, for example
`control.for`, `function.call`, `std.abs`, `trap.div-zero`, or `type.int32`. The tag vocabulary
is deliberately open across the corpus because the sets are case-local coverage labels, not HIR
rejection tags: a case tag never changes interpretation, and a tag not understood by the harness
is reported, not treated as a construct to interpret. The reader rejects a non-lowercase tag, an
empty array element, and an unsorted or duplicated list so the descriptor text is deterministic.

### 3.4 Validation summary

The reader rejects, with a diagnostic naming the field:

- an unknown `format`, `formatVersion`, `semanticsVersion`, or `profile`;
- a missing required field, a wrong JSON type, or a negative number;
- a case name outside `[A-Za-z0-9][A-Za-z0-9._-]*`;
- an entry kind other than `program`, or a non-canonical entry name;
- a non-empty `inputs` array;
- `fuel` equal to zero;
- a feature tag outside the grammar, or a list that is not strictly sorted;
- an expected status other than `halted` or `trap`;
- a status/trap mismatch, an unknown trap name, or a `globals` element that is not an integer.

Run-time validation additionally rejects an entry name that differs from the dump program and an
expected-global count that differs from the dump's global count.

Example descriptor:

```json
{
  "format": "stc-core-case",
  "formatVersion": "0.1.0",
  "semanticsVersion": "1.0-draft",
  "profile": "stc-2013-oop-utf8-v1",
  "name": "add-01",
  "entry": {"kind": "program", "name": "P"},
  "inputs": [],
  "cycles": 1,
  "fuel": 10000,
  "features": ["arith.add", "type.int32"],
  "expected": {"status": "halted", "trap": null, "globals": [3], "cycles": 1}
}
```

## 4. Reference result `stc-sem-result` `0.1.0`

`stc-sem` writes one result object per run. The object has these members:

| Field | Type | Meaning |
| --- | --- | --- |
| `format` | string | Exactly `stc-sem-result` |
| `formatVersion` | string | Exactly `0.1.0` |
| `semanticsVersion` | string | The semantics version of the run, `1.0-draft` |
| `dumpFormatVersion` | string | The consumed dump format, `0.1.0` |
| `caseFormatVersion` | string | The consumed case format, `0.1.0` |
| `profile` | string | `stc-2013-oop-utf8-v1` |
| `case` | string | The case name from the descriptor |
| `entry` | object | `{ "kind": "program", "name": <canonical name> }` |
| `status` | string | Terminal status, §4.1 |
| `value` | integer or null | Terminal value, when the machine stopped at one |
| `trap` | string or null | Trap name for `status = "trap"` |
| `trapSpan` | span or null | Span of the trapping construct |
| `globals` | array | Terminal global snapshot, §4.2 |
| `cycles` | non-negative integer | Completed cycles |
| `requestedCycles` | non-negative integer | Cycle count requested by the case |
| `fuel` | positive integer | Fuel bound used by the run |
| `trace` | array or absent | Per-step trace, present only with `--trace`, §4.3 |

### 4.1 Terminal status

| Status | Meaning |
| --- | --- |
| `halted` | The machine reached its halt focus with no terminal value |
| `trap` | The machine stopped on a core trap; `trap` and `trapSpan` are non-null |
| `value` | The machine stopped at a value focus; `value` is non-null |
| `outOfFuel` | The fuel bound was exhausted before a terminal state |
| `stuck` | A structural precondition failed; this is a reference-model fault |

`value` is defensive: the source machine's top-level transitions halt rather than expose a bare
value, but the result format records one if a run ever ends at a value focus. Only `halted` and
`trap` are valid expectations for a corpus case; `outOfFuel` and `stuck` always fail comparison
and identify either an insufficient `fuel` bound or a defect in the reference path.

### 4.2 Global snapshot

`globals` has one object per dump global in `unit.globals` order:

| Field | Type | Meaning |
| --- | --- | --- |
| `name` | string | Canonical global name |
| `type` | type object | Declared type in the dump's type shape |
| `value` | integer | Terminal value |

The type object uses the HIR-dump scalar shapes: `{"kind":"bool"}`, `{"kind":"int","width":…,
"signed":…}`, `{"kind":"bits","width":…}`, and `{"kind":"char","charKind":…}`. Values are exact
integers in the modeled representation, not scaled to the declared width; the dump and machine
read them through the declared type.

### 4.3 Trace

With `--trace`, `trace` is an array with one entry per machine step, recorded **before** the
step runs, so the last entry describes the pre-terminal focus. Without `--trace` the member is
absent rather than empty, so an untraced result is smaller and a consumer can distinguish "no
trace requested" from "zero steps".

| Field | Type | Meaning |
| --- | --- | --- |
| `step` | non-negative integer | Zero-based step index |
| `cycles` | non-negative integer | Completed cycles before the step |
| `cyclesLeft` | non-negative integer | Requested cycles remaining before the step |
| `focus` | string | `expr`, `value`, `stmts`, `resume`, `returned`, `trapped`, or `halted` |
| `frames` | non-negative integer | Frames on the call stack |
| `continuations` | non-negative integer | Continuations in the running frame |

A trace is diagnostic evidence for triage; it is not compared against the Rust artifact.

Example trapped result:

```json
{"format":"stc-sem-result","formatVersion":"0.1.0","semanticsVersion":"1.0-draft",
 "dumpFormatVersion":"0.1.0","caseFormatVersion":"0.1.0","profile":"stc-2013-oop-utf8-v1",
 "case":"div-zero-01","entry":{"kind":"program","name":"P"},"status":"trap",
 "value":null,"trap":"DIV_ZERO","trapSpan":{"file":"case.st","start":85,"end":95},
 "globals":[{"name":"G","type":{"kind":"int","signed":true,"width":32},"value":0}],
 "cycles":0,"requestedCycles":1,"fuel":10000}
```

The example is split across lines for readability; `stc-sem` emits one line.

## 5. Command-line interface

```text
lake exe stc-sem --dump <dump.json> --case <case.json> [--trace] [--output <path>]
```

`--dump` and `--case` are required exactly once; `--output` writes the result to a file instead
of stdout; `--trace` adds the trace; `--help` and `-h` print the usage line. A repeated
`--dump`, `--case`, or `--output`, an unknown argument, and a missing path are usage errors.
Diagnostics go to stderr, and stdout carries only the result object so a pipe is
machine-readable.

| Exit | Meaning |
| --- | --- |
| 0 | A result was produced and written, including `trap`, `outOfFuel`, or `stuck` |
| 2 | Invalid command line, unreadable input, or unwritable output |
| 3 | Invalid dump or case format/version/schema, an entry-name mismatch, or a global-count mismatch |

Exit `0` for a trap is deliberate: a trap is an observation to compare, not a command failure.
A harness must not treat the process status as the case verdict; it reads `status` from the
result.

## 6. Result comparison

The harness compares a result against a descriptor's `expected` object on exactly four
observations:

- `status` equals the expected status;
- `trap` equals the expected trap, including both being null;
- the global values, in declaration order, equal the expected `globals`;
- `cycles` equals the expected cycle count.

The case name, entry point, versions, and profile are provenance; they are checked when reading,
not compared as observations. An entry name that differs from the dump program is an input error
(exit `3`), never a mismatch. The optional trace is diagnostic and is not part of the comparison.

Comparison against the compiled Rust artifact uses the same four observations; its
`requestedCycles` and fuel are harness inputs and are not Rust program inputs. The compiled
artifact's global snapshot is compared by declaration position and value, which is why the dump
fixes global order.

## 7. Corpus, coverage inventory, exclusions, and gating

A corpus case is the directory `tests/verification/semantics/<case>/` containing `case.json` and
the core source file or files the dump is produced from. The harness derives the dump with
`stc hir-dump` and runs `stc-sem` against it; neither the case descriptor nor the reference
interpreter reads source text. Most cases have one `.st` source and one cycle; the cycle-boundary
cases request two or three cycles and trap in a later cycle.

The corpus holds 74 cases. The committed report (§8.2) is the per-case result record: it lists
every case, its feature tags, and the four compared observations for the descriptor, the reference
interpreter, and the compiled artifact. This section records the coverage inventory that maps each
claimed core item to those cases, the justified exclusions, and the disagreement triage log.

### 7.1 Coverage inventory

The inventory covers the P0.6 claim in [Plan-Phase0.md](Plan-Phase0.md) §3.3 and the exact
standard functions and conversion categories in [Semantics.md](Semantics.md) §3.1-§3.2. Each row
names at least one case; one case can support several rows. Tags are coverage labels (§3.3), not
constructs the harness interprets.

#### Types

| Claimed item | Cases |
| --- | --- |
| `BOOL` | `bool-bitwise`, `bool-comparison`, `comparison-operators`, `conversion-bool`, `conversion-bool-bits`, `global-defaults`, `globals-order`, `mutual-recursion`, `std-sel-bool`, `unary-operators` |
| `SINT` | `case-signed-labels`, `comparison-signed-values`, `conversion-narrow-ok`, `conversion-reinterpret`, `conversion-widen`, `global-defaults`, `std-min-max-limit-sint`, `trap-overflow-abs`, `trap-overflow-mul`, `trap-overflow-neg`, `trap-overflow-sub`, `trap-range-narrow`, `type-signed-max` |
| `INT` | `conversion-bool`, `conversion-int-bits`, `conversion-reinterpret`, `global-defaults`, `global-initializers`, `trap-range-int-to-byte`, `type-signed-max` |
| `DINT` | `arith-sub-mul`, `control-flow`, `division-truncating`, `global-defaults`, `pow-operator`, `program-field-persistence`, `std-functions`, `std-mux-selectors`, `trap-overflow-mod-min`, `trap-overflow-pow`, `unary-operators`, `value-result` (and most remaining cases) |
| `LINT` | `arith-lint`, `conversion-lint-ulint`, `global-defaults`, `trap-overflow-lint-add`, `type-signed-max` |
| `USINT` | `arith-unsigned-div`, `arith-unsigned-widths`, `case-unsigned-labels`, `comparison-signed-values`, `conversion-reinterpret`, `conversion-widen`, `global-defaults`, `trap-overflow-unsigned-add`, `trap-range-dint-to-usint`, `type-unsigned-max` |
| `UINT` | `arith-unsigned-widths`, `global-defaults`, `type-unsigned-max` |
| `UDINT` | `arith-unsigned-widths`, `conversion-reinterpret`, `conversion-widen`, `global-defaults`, `type-unsigned-max` |
| `ULINT` | `arith-unsigned-widths`, `conversion-lint-ulint`, `global-defaults`, `trap-overflow-ulint-add`, `type-unsigned-max` |
| `BYTE` | `bitwise-wide`, `conversion-bits-widths`, `global-defaults`, `trap-range-int-to-byte`, `type-unsigned-max` |
| `WORD` | `bitwise-bits`, `comparison-bits-char`, `conversion-bits-widths`, `conversion-bool-bits`, `conversion-int-bits`, `global-defaults`, `std-min-max-bits-char`, `std-select-limit-bits`, `type-unsigned-max`, `unary-operators` |
| `DWORD` | `bitwise-wide`, `conversion-bits-widths`, `global-defaults`, `type-unsigned-max` |
| `LWORD` | `bitwise-wide`, `global-defaults`, `type-unsigned-max` |
| `CHAR` | `comparison-bits-char`, `conversion-char`, `conversion-char-wide`, `global-defaults`, `std-min-max-bits-char`, `std-select-limit-bits`, `type-char-values` |
| `WCHAR` | `conversion-char`, `conversion-char-wide`, `global-defaults`, `std-min-max-bits-char`, `type-char-values` |
| `UCHAR` | `conversion-char`, `conversion-char-wide`, `global-defaults`, `type-char-values` |

#### Literals, expressions, and operators

| Claimed item | Cases |
| --- | --- |
| Boolean, integer, typed bit-string, and character literals | `bool-bitwise`, `type-signed-max`, `type-unsigned-max`, `type-char-values`, `conversion-bits-widths`, `conversion-char` |
| Variable and global access; local and program fields | `value-result`, `got-access`, `function-global-write`, `program-field-persistence`, `globals-order` |
| Parenthesized expressions | `pow-operator`, `unary-operators`, `division-truncating` |
| Unary `+`, `-`, `NOT` | `unary-operators`, `trap-overflow-neg`, `bool-bitwise` |
| Arithmetic `+`, `-`, `*` | `arith-sub-mul`, `arith-lint`, `arith-unsigned-widths`, `control-flow`, `trap-overflow-*` |
| Arithmetic `/`, `MOD` | `division-truncating`, `arith-unsigned-div`, `trap-div-zero`, `trap-div-zero-mod`, `trap-overflow-mod-min` |
| Arithmetic `**` | `pow-operator`, `trap-overflow-pow`, `trap-pow-negative` |
| Bitwise `AND`, `OR`, `XOR` | `bool-bitwise`, `bitwise-bits`, `bitwise-wide` |
| Comparisons `=`, `<>`, `<`, `<=`, `>`, `>=` | `bool-comparison`, `comparison-bits-char`, `comparison-operators`, `comparison-signed-values` |
| Calls to core functions, positional and named arguments | `value-result`, `call-named-arguments`, `got-access`, `mutual-recursion`, `stmt-call-discard`, `trap-in-function` |

#### Exact standard functions

| Claimed item | Cases |
| --- | --- |
| `ABS` | `std-functions`, `trap-overflow-abs` |
| `SEL` | `std-functions`, `std-sel-bool`, `std-select-limit-bits` |
| `MIN` | `std-functions`, `std-min-max-bits-char`, `std-min-max-limit-sint` |
| `MAX` | `std-functions`, `std-min-max-bits-char`, `std-min-max-limit-sint` |
| `LIMIT` | `std-functions`, `std-min-max-bits-char`, `std-min-max-limit-sint`, `std-select-limit-bits` |
| `MUX` | `std-functions`, `std-mux-selectors` |

#### Statement forms

| Claimed item | Cases |
| --- | --- |
| Assignment | `arith-sub-mul`, `case-selector`, `for-negative-step`, `function-global-write`, `nested-if-elsif`, `value-result` (and most other cases) |
| `IF`/`ELSIF`/`ELSE` | `nested-if-elsif`, `control-flow`, `loop-control`, `mutual-recursion`, `trap-second-cycle`, `trap-third-cycle` |
| `CASE` with integer literals, ranges, and `ELSE` | `case-selector`, `case-signed-labels`, `case-unsigned-labels` |
| `FOR` with positive and negative `BY`; zero iterations | `control-flow`, `for-negative-step`, `for-zero-iterations` |
| `WHILE` | `while-false-repeat-once`, `loop-control` |
| `REPEAT` | `while-false-repeat-once`, `loop-control` |
| `EXIT` | `loop-control` |
| `CONTINUE` | `loop-control` |
| `RETURN` | `program-return`, `program-return-multi-cycle` |
| Empty statement | `empty-program`, `global-defaults` |
| Statement-position call (additional evidence; not in the Plan §3.3 list) | `stmt-call-discard` |

#### Conversions

| Claimed category | Cases |
| --- | --- |
| Identity conversion | `conversion-reinterpret` |
| Integer-to-integer widening, including signed to wider unsigned | `conversion-widen` |
| Integer-to-integer same-width reinterpretation, including 64-bit | `conversion-lint-ulint`, `conversion-reinterpret` |
| Integer-to-integer narrowing in range | `conversion-narrow-ok` |
| Integer-to-integer narrowing out of range (`RANGE`) | `trap-range-narrow`, `trap-range-int-to-byte`, `trap-range-dint-to-usint` |
| Bit-string-to-bit-string width conversion | `conversion-bits-widths` |
| Integer-to-bit-string and bit-string-to-integer | `conversion-int-bits` |
| `BOOL` to/from integer | `conversion-bool` |
| `BOOL` to/from bit string | `conversion-bool-bits` |
| Character-to-character (`CHAR`/`WCHAR`/`UCHAR`) | `conversion-char`, `conversion-char-wide` |

Character-to-integer conversion is not a claimed category: `ConversionLegal` in
[Semantics.md](Semantics.md) §3.2 excludes it and the source analyzer rejects it.

#### Traps

| Claimed trap | Cases |
| --- | --- |
| `OVERFLOW` | `trap-overflow-abs`, `trap-overflow-lint-add`, `trap-overflow-mod-min`, `trap-overflow-mul`, `trap-overflow-neg`, `trap-overflow-pow`, `trap-overflow-sub`, `trap-overflow-ulint-add`, `trap-overflow-unsigned-add` |
| `DIV_ZERO` | `trap-div-zero`, `trap-div-zero-mod`, `trap-in-function`, `trap-after-global-write-in-function`, `trap-second-cycle`, `trap-third-cycle` |
| `RANGE` | `trap-pow-negative`, `trap-range-dint-to-usint`, `trap-range-int-to-byte`, `trap-range-narrow` |

#### GOT and cell access

| Claimed item | Cases |
| --- | --- |
| Read a global through its GOT root from a function | `got-access`, `function-global-write` |
| Write a global through its GOT root from a function | `function-global-write`, `trap-after-global-write-in-function` |
| Global declaration order and default values | `globals-order`, `global-defaults`, `global-initializers` |
| Program-field storage and persistence across cycles | `program-field-persistence` (indirect, see E4) |
| No-global program | `empty-program`, `stmt-call-discard` |

#### Program and execution shape

| Claimed item | Cases |
| --- | --- |
| One program with canonical uppercase entry name | every case (all use `MAIN`) |
| One task, one sequential cycle | every single-cycle case |
| Multiple cycles with persistent program state | `program-field-persistence`, `program-return-multi-cycle`, `trap-second-cycle`, `trap-third-cycle` |
| Global constant initializers | `global-initializers` |
| `FUNCTION` with `VAR_INPUT` and a result | `value-result`, `call-named-arguments`, `got-access`, `mutual-recursion`, `trap-in-function` |
| Recursion under the interpreter fuel bound | `mutual-recursion` |
| `RETURN` ends a cycle without ending the run | `program-return`, `program-return-multi-cycle` |

### 7.2 Justified exclusions

E1 — **Result-less (procedure-style) function calls.** A call to a `FUNCTION` with no result is
source-legal and codegen supports it, but the frozen dump represents every call expression with a
`type` and a result-less call has none; `stc hir-dump` rejects it with `type.error`. Representing
it needs a dump-format change, so it is deferred with T1. Statement-position calls of a
result-bearing function are covered as additional evidence by `stmt-call-discard`.

E2 — **Direct self-recursion.** Source resolves the function name inside its body to the result
variable, so a direct recursive call is not expressible; `FUNCTION Fact` cannot call `Fact(n - 1)`.
Mutual recursion is expressible and covered by `mutual-recursion` (T3). This is a source-language
rule, not a semantics or compiler defect.

E3 — **Shift and rotate.** The core HIR has no shift or rotate operator or standard function;
`stc hir-dump` rejects `SHL`/`SHR`/`ROL`/`ROR` with `call.std-function-*`. Shift and rotate are
part of the exact target model and target IR ([Plan-Phase0.md](Plan-Phase0.md) §3.4), not the
P0.6 source corpus, so they carry no core case.

E4 — **Direct program-field root observation.** The harness compares global values only; the
snapshot surface reads manifest roots, and a program field is not a global. Program-field
persistence is observed indirectly: `program-field-persistence` mirrors the field into a global
each cycle, so the field's stored value is compared without extending the frozen report schema.

E5 — **Cross-global initializer ordering (Q5).** Source requires global initializers to be
constant expressions, so no initializer can read another global and cross-global initialization
order is not observable through the source surface. Constant initializers are covered by
`global-initializers`; Q5 remains a model decision, not a conformance claim.

E6 — **Q3 character-validity traps.** The P0.3 character model is layout-only; Unicode scalar
validity and the `WCHAR_TO_UCHAR`/`UCHAR_TO_WCHAR` traps are deferred to the string phase and
excluded from P0.6 ([Semantics.md](Semantics.md) §3.1). Corpus cases use valid values such as
`CHAR#'A'` and `WCHAR#16#20AC`, and make no lone-surrogate or above-BMP conformance claim.

E7 — **Types, statements, and traps outside the frozen core.** `REAL`/`LREAL`, time/date,
strings, arrays, structs, enums, references, pointers, OOP, function blocks, tasks, I/O, retained
state, additional standard libraries, and traps other than `OVERFLOW`, `DIV_ZERO`, and `RANGE`
are excluded by [Semantics-Charter.md](Semantics-Charter.md) §6 and Plan-Phase0 §3.3. They have
no case and no claim in this report.

### 7.3 Triage log

Every disagreement or gap found while building the corpus is recorded here with its
classification and resolution. A closed entry has a regression test or a corpus case; an excluded
entry is covered by §7.2. No reference-versus-artifact disagreement remains in the committed
corpus: all 74 cases pass against the descriptor, the reference interpreter, and the compiled
artifact.

| ID | Classification | Finding and reproduction | Resolution |
| --- | --- | --- | --- |
| T1 | Dump-format limitation | `FUNCTION Bump ... END_FUNCTION` followed by `Bump();` in a program is source-legal and runs in codegen, but `stc hir-dump` reports `type.error` for the call expression. | Excluded (E1). The dump schema carries a `type` on every call expression; adding a void-call shape requires a new dump version. `stmt-call-discard` covers a result-bearing call in statement position. |
| T2 | Rust source-analyzer bug | `MIN(X, Y)` with two `WORD` values failed with `cannot convert WORD to USINT`; `MIN`/`MAX` with `CHAR` failed with `argument of MIN must be ANY_MAGNITUDE, found CHAR`, although the Lean model's `isMagnitude` includes `CHAR`, `WORD`, and the other bit strings. | Fixed. `AnyMagnitude` now includes `CHAR`, and `std_unify` chooses a bit-string or character result for `MIN`/`MAX` instead of integer promotion. Regression test `magnitude_standard_functions_accept_bit_strings_and_characters` and corpus case `std-min-max-bits-char` pass; `LIMIT` over the same categories also passes. |
| T3 | Source-language rule | `Fact(n - 1)` inside `FUNCTION Fact` fails with `value of type DINT is not callable`; the function name denotes its result variable. | Not a defect. Mutual recursion is expressible, and `mutual-recursion` exercises recursion under the fuel bound. Excluded (E2). |
| T4 | Case-authoring error | Two early hand-written expectations were wrong: `global-initializers` expected one value for two globals, and `program-return-multi-cycle` expected the first-cycle value instead of the third-cycle value. The reference and artifact agreed with each other in both cases; only the descriptor was wrong. | Corrected from the Lean semantics, not copied from one implementation. These were descriptor defects, not reference-versus-artifact disagreements, and no semantics or compiler behavior changed. |
| T5 | Source-language rule | A global initializer that reads another global fails with `STC-E-CONST-001 name is not a constant`. | Not a defect. Recorded as E5; constant initializers are covered. |

### 7.4 Gating

The P0.0 gate in `tools/ci/check_semantics_placeholder.py` fails CI if the corpus is empty and the
harness exits zero, or if any `case.json` exists and the harness exits nonzero, so a case can
never be added to a harness that does not run it and an empty corpus can never pass. On a
non-empty corpus the same gate regenerates the report and rejects a committed report that differs
byte-for-byte (§8.2). Every case must pass or carry a written exclusion.

## 8. Conformance report `stc-sem-conformance-report` `0.1.0`

The harness command line is:

```text
lake exe semantics-conformance [--corpus DIR] [--stc PATH] [--stc-sem PATH]
                               [--runtime PATH] [--clang PATH] [--report PATH]
```

`--corpus` defaults to `tests/verification/semantics`. The tool paths fall back to `STC`,
`STC_SEM`, `STC_RUNTIME`, and `STC_CLANG`, then to known build paths, then to a `PATH` probe
for the two programs; the runtime library has no `PATH` probe. One report object is written
to stdout, and to `--report <path>` when given. Exit `0` means every case passed; exit `1`
means at least one case disagreed; exit `2` is a usage or tool failure; exit `3` means the
corpus is empty.

The report has these members:

| Field | Type | Meaning |
| --- | --- | --- |
| `format` | string | Exactly `stc-sem-conformance-report` |
| `formatVersion` | string | Exactly `0.1.0` |
| `semanticsVersion` | string | The compared semantics version, `1.0-draft` |
| `profile` | string | `stc-2013-oop-utf8-v1` |
| `dumpFormat`, `dumpFormatVersion` | string | `stc-core-hir`, `0.1.0` |
| `caseFormat`, `caseFormatVersion` | string | `stc-core-case`, `0.1.0` |
| `resultFormat`, `resultFormatVersion` | string | `stc-sem-result`, `0.1.0` |
| `snapshotFormat`, `snapshotFormatVersion` | string | `stc-runtime-snapshot`, `0.1.0` |
| `evidence` | string | Fixed text labelling the run as V0 differential evidence, not a refinement proof |
| `cases` | array | One entry per descriptor, §8.1 |
| `summary` | object | `{"total": n, "passed": n, "failed": n}` |

A passing report carries no timestamp, temporary path, or build identifier, so repeated passing
runs produce equal report bytes; a failure diagnostic may name the temporary case directory.
The report format is frozen at `0.1.0`: a
change to a field name, shape, meaning, or the feature-tag placement requires a new
`formatVersion`.

### 8.1 Case entry

Each case entry has the following members:

| Field | Type | Meaning |
| --- | --- | --- |
| `name` | string | Case name from the descriptor |
| `features` | array of strings | Coverage tags from the descriptor, in order |
| `status` | string | `pass` or `fail` |
| `failures` | array of strings | Empty on `pass`; otherwise, one line per disagreed observation |
| `expected` | object | The descriptor's expected observation |
| `reference` | object or null | The reference observation, or null when that path failed |
| `artifact` | object or null | The compiled artifact's observation, or null when that path failed |

`expected`, `reference`, and `artifact` use the four compared fields `status`, `trap`,
`globals`, and `cycles`. The artifact entry additionally has `exitCode`, the launcher's
process status (`0` normal, `4` trap), so a report distinguishes an agreeing trap from an
unexpected launcher failure that never produced an observation.

The compiled artifact is run with the opt-in, test-only `--snapshot` surface. The launcher
reads every manifest root through `Host::debug_read` after the run loop and never writes,
forces, or retargets a root; the surface is not a production debug service. The harness maps
the dump's globals to their `_ROOT::NAME` roots in declaration order and rejects a missing or
inconsistent root.

### 8.2 Committed report artifact

The report is a committed artifact at `tests/verification/semantics/conformance-report.json`; CI
does not regenerate it into the tree. A passing report carries no timestamp, path, or build
identifier (§8), so regeneration is byte-stable for the same corpus and toolchain. CI runs the
harness with `--report` into a temporary path, and `tools/ci/check_semantics_placeholder.py`
compares the bytes with the committed file. A corpus or harness change that changes any
observation fails the gate until the report is regenerated and committed. A failing report is
never committed: a disagreement is filed with its reproduction and fixed or excluded first.

Regenerate the report from the repository root after an intentional corpus change:

```text
cd lean
lake exe semantics-conformance --report ../tests/verification/semantics/conformance-report.json
```

Review the report diff together with the corpus diff; the CI gate rejects a report that does not
match the corpus.

## 9. Versioning policy

- `format` names the family and never changes within `0.x`; `formatVersion` is exact.
- The case, result, report, and snapshot `0.1.0` formats are frozen for the P0.6 core subset.
  A change to a field name, meaning, shape, canonical-name rule, ordering rule, status
  vocabulary, trap vocabulary, or comparison rule requires a new `formatVersion` for that
  format.
- Adding a feature tag to a case is compatible within `0.1.0` because tags are coverage labels.
  Adding a new status or trap name requires a version bump.
- `semanticsVersion` is independent and exact. A case whose semantics version differs from the
  reader's is rejected, not migrated; the corpus is regenerated when the semantics changes at
  P0.7.
- Consumers compare header strings exactly and never apply best-effort reading.

## 10. Changelog

| Date | Version | Change |
| --- | --- | --- |
| 2026-11-02 | `0.1.0` | P0.6 fourth milestone: expanded the corpus to 74 cases covering every claimed core type, operator and exact standard function, statement form, conversion, trap, GOT access, and program shape, or a written exclusion; recorded the coverage inventory, exclusions E1-E7, and triage log T1-T5; fixed the source analyzer's `MIN`/`MAX`/`LIMIT` magnitude typing for bit strings and characters (T2); committed the byte-stable `stc-sem-conformance-report` `0.1.0` report at `tests/verification/semantics/conformance-report.json` and made the corpus gate reject a stale copy. The case, result, report, and snapshot formats and the comparison rule are unchanged |
| 2026-11-02 | `0.1.0` | P0.6 third milestone: seeded the core corpus with `value-result`, `trap-div-zero`, `globals-order`, `control-flow`, and `got-access`; the differential harness runs all five in CI and the placeholder gate requires a non-empty passing corpus. Froze the `stc-sem-conformance-report` `0.1.0` schema, including per-case feature tags and the explicit V0 differential label. The case and result formats and the comparison rule are unchanged |
| 2026-11-01 | `0.1.0` | P0.6 second milestone: defined the `stc-core-case` descriptor and `stc-sem-result` object, froze both at `0.1.0`, recorded the CLI contract and exit codes, the four-observation comparison, the corpus layout, and the fail-closed gating rule. No corpus case is committed yet |
