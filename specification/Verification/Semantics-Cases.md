# Core Semantics Cases and Reference Results

Profile: `stc-2013-oop-utf8-v1`
Date: 2026-11-01
Semantics version: `1.0-draft`
Case format: `stc-core-case` version `0.1.0`
Result format: `stc-sem-result` version `0.1.0`
Status: P0.6 third milestone. The `stc-sem` reference-interpreter path and the differential harness
are implemented, and the seeded corpus under `tests/verification/semantics/` passes: the frozen
dump, the reference interpreter, and the compiled artifact agree on every case. This is V0
differential evidence, not a refinement proof.
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

## 7. Corpus layout and gating

A corpus case is the directory `tests/verification/semantics/<case>/` containing `case.json` and
the core source file or files the dump is produced from. The harness derives the dump with
`stc hir-dump` and runs `stc-sem` against it; neither the case descriptor nor the reference
interpreter reads source text. Each seeded case has one `.st` source and one cycle, so every
comparison includes exactly one completed cycle unless the case traps during it.

| Case | Coverage |
| --- | --- |
| `value-result` | a function call and arithmetic produce a global value; halted |
| `trap-div-zero` | a division by zero after a global write; traps with `DIV_ZERO` |
| `globals-order` | two `DINT` globals and a `BOOL` global compared in declaration order |
| `control-flow` | `FOR` and `IF` nest within a single program cycle |
| `got-access` | a function reads a global through its GOT root |

The P0.0 gate in `tools/ci/check_semantics_placeholder.py` fails CI if the corpus is empty and the
harness exits zero, or if any `case.json` exists and the harness exits nonzero, so a case can
never be added to a harness that does not run it and an empty corpus can never pass. Every case
must pass or carry a written exclusion.

## 8. Versioning policy

- `format` names the family and never changes within `0.x`; `formatVersion` is exact.
- Both `0.1.0` formats are frozen for the P0.6 core subset. A change to a field name, meaning,
  shape, canonical-name rule, ordering rule, status vocabulary, trap vocabulary, or comparison
  rule requires a new `formatVersion`.
- Adding a feature tag to a case is compatible within `0.1.0` because tags are coverage labels.
  Adding a new status or trap name requires a version bump.
- `semanticsVersion` is independent and exact. A case whose semantics version differs from the
  reader's is rejected, not migrated; the corpus is regenerated when the semantics changes at
  P0.7.
- Consumers compare header strings exactly and never apply best-effort reading.

## 9. Changelog

| Date | Version | Change |
| --- | --- | --- |
| 2026-11-02 | `0.1.0` | P0.6 third milestone: seeded the core corpus with `value-result`, `trap-div-zero`, `globals-order`, `control-flow`, and `got-access`; the differential harness runs all five and the placeholder gate requires a non-empty passing corpus. The formats, comparison rule, and versioning policy are unchanged |
| 2026-11-01 | `0.1.0` | P0.6 second milestone: defined the `stc-core-case` descriptor and `stc-sem-result` object, froze both at `0.1.0`, recorded the CLI contract and exit codes, the four-observation comparison, the corpus layout, and the fail-closed gating rule. No corpus case is committed yet |
