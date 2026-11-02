# Core HIR Dump — Format `0.1.0`

Profile: `stc-2013-oop-utf8-v1`
Date: 2026-11-01
Dump format: `stc-core-hir` version `0.1.0` (frozen by P0.6)
Semantics version: `1.0-draft`
Status: P0.6 first milestone. The format, emitter, validator, and CLI command are implemented;
the Lean reader and the differential harness are later P0.6 work.
Companion: [Semantics](Semantics.md) · [Phase 0 Implementation Plan](Plan-Phase0.md)

This document is the normative schema of the core-HIR JSON dump consumed by the reference
interpreter and the conformance harness. The dump is **V0 differential evidence, not a
refinement proof**: agreement between the reference evaluator and the Rust compiler on a corpus
supports confidence in both, but no theorem in Phase 0 states that the compiler refines the
semantics.

## 1. Purpose and non-goals

The dump is a canonical projection of the frozen core subset defined in
[Plan-Phase0.md](Plan-Phase0.md) §3.3–§3.4 and [Semantics.md](Semantics.md) §1. It is the only
HIR input of `lake exe stc-sem`; the reference tools never parse source text and never consume
the Rust compiler's debug text dump.

The dump is not:

- a serialization of the Rust HIR arenas: Rust `DefId`, `ExprId`, `StmtId`, `ParamId`,
  `LocalId`, `FieldId`, and vector indices do not appear;
- a debug format: `stc_hir::dump_hir` remains a separate, unstable, human-readable text dump;
- a target-IR format: the target fragment is described in
  [Core-IR-Coverage.md](Core-IR-Coverage.md) and is not part of this document;
- a source-preserving format: comments, trivia, and original spelling are not retained.

The emitter is `stc_hir::dump_core_hir` in `crates/stc-hir/src/core_dump.rs`; the validator is
`stc_hir::verify_core_dump`; the driver command is `stc hir-dump`
(`crates/stc-driver/src/hir_dump.rs`).

## 2. Encoding, identity, and determinism

The dump is exactly one UTF-8 JSON object followed by a newline. JSON object members are emitted
in lexicographic key order by `serde_json`'s map representation; consumers must read members by
name and must not depend on member order. Numbers are JSON integers; no floating-point value is
emitted in an accepted dump.

Stable identity is semantic, never an arena index or debug rendering:

| Concept | Canonical spelling |
| --- | --- |
| Definition name | Uppercase ASCII dot-joined path, with the top-level namespace omitted (for example `LIB.F`) |
| Parameter, local, program field | Uppercase ASCII identifier (for example `VALUE`) |
| Declaration position | Zero-based index in the declaration list that the dump itself contains |
| Global identity | GOT root key `PACKAGE::DOTTED.PATH` (for example `_ROOT::G`) |
| Source location | Source path plus byte offsets, for provenance and trap reporting only |

The identifier grammar is `[A-Za-z_][A-Za-z0-9_]*` after case folding; canonical names are
uppercase. A name that cannot be normalized is replaced by a deterministic schema-valid fallback
and the dump is rejected with `name.invalid`; the fallback is never accepted as identity.

Determinism is guaranteed for a fixed HIR arena and source map. Source paths are provenance, not
identity: two checkouts of the same source produce different path strings and therefore
different dump bytes. Consumers compare observable semantics, never dump text across machines.

## 3. Top-level object

| Field | Type | Meaning |
| --- | --- | --- |
| `format` | string | Exactly `stc-core-hir` |
| `formatVersion` | string | Exactly `0.1.0` |
| `semanticsVersion` | string | Exactly `1.0-draft` |
| `profile` | string | Exactly `stc-2013-oop-utf8-v1` |
| `package` | string | Canonical root package name, uppercase (normally `_ROOT`) |
| `unit` | object | The core unit, §4 |
| `features` | array | Out-of-core feature records, §11 |
| `status` | string | `ok` when `features` is empty, otherwise `rejected` |

`status` is redundant with `features`, and the validator rejects a document whose status does
not match. An `ok` dump may be interpreted; a `rejected` dump must be refused by every consumer,
even though it remains structurally valid so the rejection can be inspected.

## 4. Unit object

| Field | Type | Meaning |
| --- | --- | --- |
| `globals` | array | Global roots, in HIR declaration order, §4.1 |
| `functions` | array | Functions, sorted by canonical name and then by definition order, §4.2 |
| `program` | object or null | The single program, §4.3 |
| `span` | span | Unit span, §5 |

`unit.span` is the program span when a program exists; otherwise the first function span;
otherwise the first global span; otherwise `{ "file": "", "start": 0, "end": 0 }`. A unit with no
program is rejected (`program.missing`); a unit with more than one program is rejected
(`program.multiple`). An accepted dump contains exactly one program.

### 4.1 Global object

| Field | Type | Meaning |
| --- | --- | --- |
| `name` | string | Canonical global name |
| `gotKey` | string | GOT root key `PACKAGE::NAME` |
| `type` | type | Declared type, §6 |
| `init` | expression or null | Initializer |
| `span` | span | Declaration span |

The GOT key is computed exactly as `stc_got::global_root_key(package, canonical_name)`, with the
package and each path segment uppercased. Initializers are evaluated in unit declaration order,
which the array preserves.

### 4.2 Function object

| Field | Type | Meaning |
| --- | --- | --- |
| `name` | string | Canonical function name |
| `result` | type or null | Result type; `null` for a procedure-style function |
| `params` | array | `VAR_INPUT` parameters in signature order |
| `locals` | array | `VAR` locals in body declaration order |
| `body` | array | Statements, §7 |
| `span` | span | Declaration span |

Parameter objects have `name`, `type`, and `span`. Local objects have `name`, `type`, `init`
(expression or null), and `span`. Parameters and locals are referenced by their index in these
arrays.

### 4.3 Program object

| Field | Type | Meaning |
| --- | --- | --- |
| `name` | string | Canonical program name |
| `locals` | array | Program fields in declaration order |
| `body` | array | Cycle body statements, §7 |
| `span` | span | Declaration span |

Program fields use the local-object shape. A reference to a scalar program field lowers to a
variable place with `varKind: "local"` and the field's position in `program.locals`. A bare
`THIS` outside such a field access is rejected (`place.this`).

## 5. Span object

| Field | Type | Meaning |
| --- | --- | --- |
| `file` | string | Source path as recorded in the source map |
| `start` | non-negative integer | Inclusive byte offset |
| `end` | non-negative integer | Exclusive byte offset, not less than `start` |

Spans are carried on every declaration, statement, expression, place, call callee, CASE label,
and feature record so the reference interpreter can report the trapping construct. They do not
participate in identity. A span that names a file absent from the source map is tagged
`span.unknown-file` and emitted with an empty `file` string.

## 6. Type object

| `kind` | Extra fields | Core meaning |
| --- | --- | --- |
| `bool` | — | `BOOL` |
| `int` | `width` (8, 16, 32, 64), `signed` (boolean) | Signed or unsigned integer |
| `bits` | `width` (8, 16, 32, 64) | Bit string |
| `char` | `charKind`: `char`, `wchar`, or `uchar` | Character layout |
| `unsupported` | `feature` | A type outside the core; the dump is rejected |

Any other type, including non-core widths, `REAL`, `TIME`, date/time, strings, arrays,
references, pointers, named types, `NULL`, and generic types, lowers to `unsupported` with one
of the `type.*` tags in §11.4. An unsupported type node is structurally valid but can appear only
in a rejected dump.

## 7. Statement object

Every statement carries `kind` and `span`.

| `kind` | Fields | Meaning |
| --- | --- | --- |
| `assign` | `place`, `value` | Assignment to a resolved place |
| `call` | `call` | A statement-position call; `call` must itself be a `call` expression |
| `return` | — | `RETURN` |
| `exit` | — | `EXIT` |
| `continue` | — | `CONTINUE` |
| `empty` | — | Empty statement |
| `if` | `arms`, `else` | `IF`/`ELSIF`/`ELSE`; each arm has `cond`, `body`, `span` |
| `case` | `selector`, `arms`, `else` | `CASE`; each arm has `labels`, `body`, `span` |
| `for` | `var`, `start`, `stop`, `step`, `body` | `FOR` loop |
| `while` | `cond`, `body` | `WHILE` loop |
| `repeat` | `body`, `cond` | `REPEAT`/`UNTIL` loop |
| `unsupported` | `feature` | A statement outside the core; the dump is rejected |

`for.var` is a place. `for.step` is always an expression: the Rust checker materializes the
default step `1` as a literal, and the validator rejects a null step. CASE labels have `low`
(integer), `high` (integer or null for a single label), and `span`; only integer, boolean, and
character labels lower, and any other label is tagged `case.label`.

## 8. Expression and place object

Expressions carry `kind` and `span`. Literal, unary, binary, call, and cast expressions also carry
`type`; a variable place carries its declaration identity instead. An unsupported node carries
`feature` and `span`.

| `kind` | Fields | Meaning |
| --- | --- | --- |
| `literal` | `value` | Integer payload; `type` is core scalar |
| `var` | `varKind`, `name`, optional `position`, optional `gotKey` | Resolved variable or result place |
| `unary` | `op`, `operand` | `neg`, `pos`, `not` |
| `binary` | `op`, `lhs`, `rhs` | See the operator table |
| `call` | `callee`, `args` | Function or standard-function call, §9 |
| `cast` | `operand`, `to`, `castKind` | Explicit or implicit conversion |
| `unsupported` | `feature` | An expression outside the core; the dump is rejected |

Binary operators are `add`, `sub`, `mul`, `div`, `mod`, `pow` (power), `and`, `or`, `xor`,
`eq`, `ne`, `lt`, `le`, `gt`, and `ge`. Unary operators are `neg`, `pos` (unary plus), and
`not`. Cast kinds are `implicit` and `explicit`.

Variable places have `varKind`:

| `varKind` | Fields | Position space |
| --- | --- | --- |
| `param` | `name`, `position` | Function `params` array |
| `local` | `name`, `position` | Enclosing function `locals` or program `locals` |
| `global` | `name`, `position`, `gotKey` | Unit `globals` array |
| `result` | `name` | The enclosing function's result variable; no position |

Literal `value` is an integer for `BOOL` (0 or 1), integer types, bit strings, and characters.
A payload outside its declared type is tagged `literal.out-of-range`. Real, string, time, date,
enumeration, and null literals lower to `unsupported` nodes with the corresponding `literal.*`
tag; such dumps are rejected.

An assignment target that is not a resolved place is tagged `stmt.place-expected`. Aggregate,
field, subscript, dereference, bit-field, direct-address, instance-field, `THIS`, and foreign
declaration references lower to `unsupported` places with the `place.*` tags in §11.4.

## 9. Call object

A call expression has `kind: "call"`, `callee`, `args`, `type`, and `span`.

| Callee `kind` | Fields | Meaning |
| --- | --- | --- |
| `function` | `name`, `span` | A core function in `unit.functions` |
| `std` | `name`, `span` | A core standard function |

Function arguments are emitted in **formal-parameter order**, not source order. The checker
records each argument's formal parameter; the emitter reorders by the target signature and
requires exactly one input value for every parameter. A missing, duplicate, unknown, output, or
in-out binding is tagged (`call.argument-formal`, `call.argument-duplicate`, `call.arity`,
`call.output-binding`, `call.missing-value`) and the dump is rejected. Defaulted parameters are
outside the core (`param.default`) because the Lean call shape does not apply defaults, so an
omitted defaulted argument cannot be silently accepted.

Only these standard functions are core: `ABS`, `SEL`, `MIN`, `MAX`, `LIMIT`, and `MUX`. Every
other standard function lowers to an `unsupported` expression with
`call.std-function-<name>`. Method, super-method, interface-method, function-block, super-body,
and conversion callees are tagged `callee.*` or `call.conversion` and rejected; a typed
conversion is expected to have been lowered to a `cast` expression by the checker.

An accepted dump's function callee names must resolve to a function in `unit.functions`; the
validator enforces this. An unresolved target is tagged `call.target-missing` and rejected.

## 10. Canonical lowering decisions

These choices fix how the Rust HIR maps onto the Lean core unit. They are part of the `0.1.0`
contract.

| HIR construct | Dump representation |
| --- | --- |
| Top-level namespace wrapper | Omitted from canonical names; nested namespaces joined with `.` |
| Namespace/alias/subrange/enum/named-value/struct/class/interface/method/function-block/configuration declarations | Not emitted; tagged `def.<kind>` |
| Program scalar field | `program.locals` entry; accesses use `varKind: "local"` |
| Function result variable | `varKind: "result"` with the function's canonical name |
| Omitted `FOR` step | Materialized literal `1`; `step` is never null |
| Conversion call | `cast` expression with `castKind` |
| Source-order call arguments | Reordered to formal order |
| Synthetic standard-prelude definitions (zero-length span) | Not emitted and not tagged |
| Test-only definitions | Tagged `unit.test-only` |
| Definitions from non-root packages | Tagged `unit.external-package` once per package |

The emitter does not depend on the GOT crate; `got_root_key` is a local mirror of
`stc_got::global_root_key` and a driver test checks parity for top-level globals. Namespaced
functions are checked against `stc_got::normalize_name_path`.

## 11. Feature tags and rejection

### 11.1 Rejection rule

Every construct outside the frozen core produces at least one feature record:

```json
{"tag": "type.real", "detail": "REAL and LREAL are outside the core", "at": {"file": "...", "start": 0, "end": 0}}
```

`at` is a span or null. Records are sorted by `(tag, file, start, end, detail)` and duplicates
are removed, so the feature array is deterministic. A non-empty feature array sets
`status: "rejected"`. Consumers must reject the dump; they must not skip the construct, guess a
default, or interpret the surrounding unit.

A consumer must also reject an unknown `format`, `formatVersion`, `semanticsVersion`, or
`profile`, a status/features mismatch, a missing required field, a malformed type, span, place,
expression, statement, or callee, a non-core width or character kind, a variable position outside
its declaration list, a result reference outside a function, a global key without `::`, a
duplicate declaration name, a null `for.step`, or a function callee that does not resolve.
Nesting deeper than 512 nodes is rejected to bound validator work.

### 11.2 Unit and declaration tags

`package.invalid`, `unit.external-package`, `unit.test-only`, `def.alias`, `def.subrange`,
`def.enum`, `def.named-value`, `def.struct`, `def.function-block`, `def.class`, `def.interface`,
`def.method`, `def.configuration`, `def.anonymous`, `name.invalid`,
`name.non-namespace-scope`, `function.duplicate-name`, `function.no-body`,
`function.body-owner`, `function.body-params`, `param.duplicate-name`, `param.owner-mismatch`,
`param.direction`, `param.default`, `local.duplicate-name`, `local.owner-mismatch`,
`local.temp`, `local.retain`, `local.external`.

### 11.3 Global, program, and statement tags

`global.section`, `global.retain`, `global.constant`, `global.direct-address`, `global.io`,
`global.duplicate-name`, `program.missing`, `program.multiple`, `program.var-access`,
`program.no-body`, `program.body-owner`, `program.body-param`, `program.body-local`,
`program.field-duplicate-name`, `program.field-owner`, `program.field-section`,
`program.field-constant`, `program.field-at`, `program.field-io`, `program.field-retain`,
`stmt.ref-assign`, `stmt.attempt`, `stmt.place-expected`, `case.label`.

### 11.4 Type, place, expression, and call tags

Type tags: `type.error`, `type.real`, `type.time`, `type.date`, `type.string`, `type.array`,
`type.ref`, `type.ptr`, `type.named`, `type.null`, `type.generic`, `type.width`.

Place tags: `place.this`, `place.field`, `place.index`, `place.deref`, `place.bit-field`,
`place.direct`, `place.instance-field`, `place.foreign-local`, `place.foreign-param`,
`place.foreign-global`, `place.result`.

Expression and literal tags: `expr.ref`, `expr.address-of`, `expr.size-of`,
`expr.pointer-conversion`, `expr.null`, `expr.array-init`, `expr.struct-init`, `expr.error`,
`literal.real`, `literal.string`, `literal.time`, `literal.date`, `literal.enum`,
`literal.null`, `literal.out-of-range`, `literal.unsupported`.

Call tags: `call.target-missing`, `call.std-function-missing`, `call.argument-formal`,
`call.argument-duplicate`, `call.arity`, `call.output-binding`, `call.missing-value`,
`call.conversion`, `callee.method`, `callee.super-method`, `callee.interface-method`,
`callee.fb`, `callee.super-body`, and one `call.std-function-<name>` per non-core standard
function. The non-core names are: `sqrt`, `ln`, `log`, `exp`, `sin`, `cos`, `tan`, `asin`,
`acos`, `atan`, `atan2`, `expt`, `trunc`, `add`, `sub`, `mul`, `div`, `mod`, `move`, `shl`,
`shr`, `rol`, `ror`, `and`, `or`, `xor`, `not`, `gt`, `ge`, `eq`, `le`, `lt`, `ne`, `len`,
`len-max`, `len-code-unit`, `left`, `right`, `mid`, `concat`, `insert`, `delete`, `replace`,
`find`, `assert`, `clock-advance`, `io-set-input`, and `io-get-output`.

Span tags: `span.unknown-file`. Test-only and external-package definitions are tagged once, at
unit granularity, with `at: null` for the external-package record.

## 12. Driver interface

`stc hir-dump <file>...` analyzes one in-memory package (input paths sorted, file order not
semantic) and writes the dump to stdout. Diagnostics go to stderr so stdout is the only payload.
`--relaxed` selects the relaxed-semicolon parse mode. `--output <path>` writes the dump to a
file instead. `stc hir-dump --verify <dump.json>` validates a dump without analyzing source;
`--message-format=json` makes the verify summary a single JSON object.

| Exit | Emit mode | Verify mode |
| --- | --- | --- |
| 0 | Dump written and accepted | Dump accepted |
| 1 | Dump written and rejected, or source analysis failed | Dump rejected (feature tags) |
| 2 | Input unreadable or output unwritable | Dump unreadable, invalid JSON, unknown format/version/profile, or malformed schema |
| 3 | Internal self-check failure (an emitter/validator bug) | — |

Emit mode self-checks every dump with the validator before writing it; a validator error is an
internal error, not a normal rejection. Rejected dumps are still written because the harness
needs to report the feature tags. `--output` and `--verify` are mutually exclusive, verify mode
accepts no input files, and emit mode requires at least one input file.

## 13. Versioning policy

- `format` names the family and never changes within `0.x`; `formatVersion` is exact.
- `0.1.0` is frozen for the P0.6 core subset. The P0.6 milestone defines it; it is not an
  unstable draft label.
- A change to a field name, field meaning, node shape, canonical-name rule, position rule,
  ordering rule, or accepted core construct requires a new `formatVersion` (`0.2.0` for an
  incompatible or contract-changing revision during Phase 0).
- Adding a feature tag for a construct that was previously untagged is compatible within
  `0.1.0` only when the construct was already rejected; because every non-empty feature list is
  rejected, this cannot turn a rejected dump into an accepted one. Removing or renaming a tag,
  or narrowing when a tag applies, requires a version bump.
- `semanticsVersion` is independent and exact. The dump format does not change when the
  semantics version changes, but every report records both.
- Consumers compare the header strings exactly. A consumer that understands only `0.1.0` must
  reject `0.1.1` and `0.2.0`; it must never apply best-effort reading.

## 14. Changelog

| Date | Format | Change |
| --- | --- | --- |
| 2026-11-01 | `0.1.0` | P0.6 first milestone: defined and froze the core-HIR dump, implemented the emitter and validator in `stc-hir`, added `stc hir-dump`, and recorded the schema here. Replaces the `0.1-draft` placeholder in Semantics.md §9.2 |
