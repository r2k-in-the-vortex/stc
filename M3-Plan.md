# M3 Implementation Plan — Name Resolution and Type Checking

Profile: `stc-2013-oop-utf8-v1`
Date: 2026-10-24
Inputs: the M3 handover (historical; see git history), `Specification/AST/AST.md` §5–§8,
`Specification/Architecture/Architecture.md` §2–§5, `Specification/Decisions.md`.

The handover's recommended architecture and order are used. M2 is committed (`24da2f7`),
so the M3 diff starts clean.

## 1. Deliverables

| Area | Deliverable |
| --- | --- |
| Crate `stc-hir` | Arenas and stable IDs, Interner, type arena, HIR nodes, HIR dump |
| Crate `stc-sema` | Collection, namespace/`USING` scopes, resolution, type checking, constant evaluation, lowering |
| Driver | `stc check <file>...` for multi-file sessions; `stc parse` stays single-file |
| Diagnostics | Stable M3 code table, snapshot-tested, actionable spans/notes |
| Tests | Positive/negative cases per rule family, multi-file namespace case, deep-chain test, traceability table |
| Fuzzing | `fuzz_analyzer` target |
| Documentation | Decisions D-059–D-075, status updates, code table |

No codegen. `stc-codegen` is not added in M3.

## 2. Crate shape

```text
crates/stc-hir/
  src/{id,interner,ty,hir,dump}.rs
crates/stc-sema/
  src/{lib,codes,session,collect,types,prelude,resolve,consteval,check,config}.rs
```

Dependency direction:

```text
stc-span <- stc-hir <- stc-sema <- stc-parser/stc-syntax
stc-diag <- stc-sema
```

`stc-hir` does not depend on `stc-syntax` or `stc-parser`; all CST knowledge stays in
`stc-sema`. HIR owns all spans and IDs and never references CST nodes.

## 3. HIR model

- Dense arenas: `defs`, `fields`, `params`, `locals`, `bodies`, `exprs`, `stmts`,
  `packages`, `std_fns`, configs.
- IDs: `DefId`, `FieldId`, `ParamId`, `LocalId`, `BodyId`, `ExprId`, `StmtId`,
  `TyId`, `StdFnId`.
- `Ty`: builtin scalars, structural arrays/refs/strings, `Named(DefId)` for nominal
  enum/subrange/named-value/struct/class/FB/interface/function/program types,
  `Null` (internal), `Generic` (standard signatures), and `Error`.
- Expression trees are arena-backed with `ExprId`, so a 20k-term binary chain is a flat
  arena chain and lowering/type checking never recurses down the left spine.
- `Cast` nodes are explicit (`Implicit`/`Explicit`); lowering never inserts a conversion
  silently and never re-resolves a name.
- Bodies carry params/locals and statement IDs; statements carry expression IDs.
- HIR dump is deterministic (ID order) and snapshot-testable.

## 4. Analysis pipeline

1. **Session/prelude**: one package (`PackageId(0)`) and session root namespace `_root`;
   intern all identifiers; install elementary types and the standard signature table.
2. **Collection**: deterministic walk (FileId order, then source order). Build namespace
   scopes, namespace defs, type defs, POU/method defs, global vars, configurations;
   record `USING`; detect duplicate definitions (report every duplicate) and invalid
   nesting; register `{stc:test}` POUs for test-only diagnostics.
3. **Type resolution**: resolve all declared types (forward references allowed), inline
   arrays/structs/strings/refs, aliases, subranges/enums (constant evaluation), fields,
   params, locals, globals. Detect alias cycles.
4. **POU graph**: resolve `EXTENDS`/`IMPLEMENTS`, detect inheritance cycles, check
   `FINAL`/`ABSTRACT`, overrides, interface completeness, assign stable vtable slots.
5. **Initializers**: constant-evaluate variable/field/param initializers; lower array and
   struct initializers; apply defaults; enforce constant-expression rules.
6. **Bodies**: scoped resolution (`locals/params` → result name → members/inherited →
   namespace/`USING` → root → prelude → builtins), type-check and lower statements and
   expressions, insert casts, resolve callees/places, check access control.
7. **Configuration**: validate configurations/resources/tasks/program instances,
   bindings, `VAR_ACCESS`, `VAR_CONFIG`.
8. **Final checks**: retain policy, `AT`/I/O rules, assignment attempt/reference rules.

If a file has parser/lexer errors, `stc check` skips semantic analysis (D-070) to avoid
cascades from recovered trees. `stc parse` is unchanged.

## 5. Multi-file session

`stc check <file>...`:

- driver sorts paths lexicographically, assigns `FileId`s in that order;
- lexes/parses every file into one `SourceMap`; diagnostics share one sink;
- sema receives all `ParsedFile`s and builds one HIR crate;
- files without `NAMESPACE` contribute to `_root`; identical namespace names merge;
- duplicates across files are errors; file order is not semantic.

M6 replaces `_root` and the implicit package with the manifest package/alias.

## 6. Semantics fixed in M3

- Elementary types, aliases, subranges, enums, named-value types, arrays (static and
  conformant), structs (including `OVERLAP` shape), strings (`STRING`/`WSTRING`/`USTRING`),
  references, class/FB/interface/function/program types.
- Case-insensitive exact-name lookup; original spelling for messages.
- Access: `PUBLIC`, `PROTECTED`, `PRIVATE`, `INTERNAL`; class/FB variables default
  `PROTECTED`, methods default `PUBLIC`; single package means `INTERNAL` is not a
  cross-package error in M3.
- `THIS`/`SUPER` restricted to class/FB/method contexts; `THIS` is not an arbitrary
  instance prefix.
- Calls: functions (overload sets), FB instance invocation, methods, interface methods,
  `SUPER()`, positional/named/output arguments, `REF(...)`, `NULL`, conversion
  functions, standard functions.
- Conversions: implicit numeric widening/narrowing (checked in M4), int→real, same-kind
  string/char, enum/subrange base rules, class/FB/interface upcasts, ref covariance,
  `NULL`; explicit-only conversions produce `STC-E-CONV-002`. Details in D-064.
- Constant evaluation: checked `i128` integer arithmetic; overflow/zero division and
  invalid bounds are diagnostics; initializers are constant unless they are aggregate
  or `REF` initializers.
- Retain: effective per-declaration policy, invalid-location and non-persistable-leaf
  diagnostics, `AT` retain warning.
- I/O: `AT` size/area/type checks, `%I` writes, wildcard executable use, `stc:io`
  payload shape (`STC-E-IO-004`).
- Test-only units: production `stc check` excludes `{stc:test}` POUs from analysis and
  reports references as `STC-E-TEST-001`; full `stc test`/assert support stays M6
  (the parser still rejects `ASSERT(...)` as `STC-E-GRAM-104`).

## 7. Diagnostic code table (M3 additions)

| Code | Meaning |
| --- | --- |
| `STC-E-NAME-001` | unknown name |
| `STC-E-NAME-003` | duplicate definition |
| `STC-E-NAME-004` | inaccessible definition (access violation) |
| `STC-E-NAME-005` | name used in an invalid context |
| `STC-E-NAME-006` | ambiguous name |
| `STC-W-NAME-002` | user POU shadows a standard name |
| `STC-E-TYPE-001` | type mismatch |
| `STC-E-TYPE-002` | invalid operand |
| `STC-E-TYPE-003` | invalid or unsupported type |
| `STC-E-TYPE-012` | assignment mismatch (preserved example) |
| `STC-E-CONST-001` | expression is not constant |
| `STC-E-CONST-002` | constant evaluation failed (overflow/div-zero/bounds) |
| `STC-E-CONST-003` | duplicate or overlapping case label |
| `STC-E-CONV-001` | no conversion exists |
| `STC-E-CONV-002` | implicit conversion is not allowed; explicit required |
| `STC-E-CALL-001` | unknown or invalid callee |
| `STC-E-CALL-002` | argument count/required argument |
| `STC-E-CALL-003` | named/output argument error |
| `STC-E-CALL-004` | overload ambiguity |
| `STC-E-OOP-001` | inheritance cycle |
| `STC-E-OOP-002` | invalid override/hiding |
| `STC-E-OOP-003` | incomplete interface implementation |
| `STC-E-OOP-004` | abstract rule violation |
| `STC-E-OOP-005` | final rule violation |
| `STC-E-OOP-006` | invalid assignment target/attempt relation |
| `STC-E-ARRAY-001` | array bound or index error |
| `STC-E-ARRAY-002` | conformant array placement |
| `STC-E-REF-001` | reference rule violation |
| `STC-E-REF-002` | null/invalid reference use |
| `STC-E-REF-003` | invalid assignment attempt |
| `STC-E-INIT-001` | initializer shape/type error |
| `STC-E-CONFIG-001` | configuration/task/program binding error |
| `STC-E-IO-001` | `AT` address/size/type mismatch |
| `STC-E-IO-002` | write to `%I` |
| `STC-E-IO-004` | malformed/non-object `stc:io` payload |
| `STC-E-RETAIN-001` | retained leaf is not persistable |
| `STC-E-RETAIN-002` | invalid RETAIN location |
| `STC-W-RETAIN-001` | RETAIN on direct `AT` variable is ignored |
| `STC-E-TEST-001` | production code references a test-only unit |

Existing lexer/parser/pragma/IO codes are preserved.

## 8. Test plan

- One positive and one negative case per rule family; every code above appears in a
  negative traceability case.
- Diagnostic snapshots assert code, severity, primary span, labels, notes, suggestions.
- Multi-file: root file plus two files sharing `NAMESPACE ProjectDep`; cross-file
  duplicate; `USING`; unknown name.
- Deep chain: 20k-term left-associative expression analyzes without stack overflow.
- `stc check` driver tests: multi-file, JSON contract, semantic error exit code,
  parse-error skip.
- `fuzz_analyzer`: tokenize/parse/analyze arbitrary bytes; sink bounded; HIR invariant
  check; no panic.

## 9. Order of work

1. This plan + decisions D-059–D-075.
2. `stc-hir` crate (IDs, interner, types, HIR, dump).
3. `stc-sema` skeleton + collection.
4. Type resolution + prelude.
5. Resolution and body checking/lowering.
6. Constant evaluation, initializers, config, IO/retain.
7. Driver integration and existing-test updates.
8. Semantics tests, snapshots, traceability, deep chain.
9. `fuzz_analyzer`, docs/status, full CI-equivalent checks.

## 10. Status (implemented)

- [x] `stc-hir` (arenas, IDs, interner, canonical types, HIR, deterministic dump).
- [x] `stc-sema` collection, namespaces/`USING`, duplicates, invalid nesting, test-only exclusion.
- [x] Type resolution/canonicalization, standard prelude, aliases, enums, subranges, arrays,
      structs, strings, references, classes/interfaces/FBs/functions/programs.
- [x] Name resolution, shadowing, qualified names, `THIS`/`SUPER`, access control, overloads.
- [x] Type checking, conversions, assignment/attempt/reference rules, operators, conditions,
      loops, arrays, initializers, OOP inheritance/override/abstract/final/interfaces.
- [x] Constant evaluation for bounds, enum values, case labels, task attributes, initializers.
- [x] Configuration/resource/task/program-instance bindings, `VAR_ACCESS`, `VAR_CONFIG`.
- [x] I/O (`AT`, `%I`, wildcard, `STC-E-IO-004`) and retention diagnostics.
- [x] `stc check <file>...` multi-file bridge; `stc parse` remains single-file.
- [x] Diagnostic snapshots, traceability table, multi-file tests, deep-chain test, `fuzz_analyzer`.
- [x] No codegen; `stc-codegen` is not part of the workspace.
- [x] `cargo test --workspace`, clippy `-D warnings`, `cargo fmt --check`, `check_docs.py`,
      Markdown lint, and the fuzz analyzer smoke run pass.
