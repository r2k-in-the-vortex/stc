# Core Syntax Coverage — P0.1

Profile: `stc-2013-oop-utf8-v1`
Date: 2026-10-27
Status: Approved for P0.1 review (2026-10-27). This table is the traceability record from the core
productions in [Grammar](../Syntax/Grammar.md) and the lexical rules in
[Tokenization](../Syntax/Tokenization.md) to the Lean definitions in `Stc.Syntax` and
`Stc.Grammar`. It makes no parser-correctness claim; Phase 4 owns the parser contracts.
Parent: [Phase 0 Implementation Plan](Plan-Phase0.md) §6 (P0.1)
Semantics reference: [stc Semantics — Phase 0 Core](Semantics.md)

## 1. Scope

The covered source subset is frozen by [Plan-Phase0.md](Plan-Phase0.md) §3.3 and the
[charter's exclusion register](Semantics-Charter.md#6-frozen-core-and-exclusion-register). The
Lean modules are:

| Module | Contents |
| --- | --- |
| [`Stc.Syntax.Tokens`](../../lean/Stc/Syntax/Tokens.lean) | `FileId`, `Span`, `ElementaryType`, `CharType`, `Keyword`, `Punct`, `TokenKind`, `Token`, `TriviaKind`, `Trivia`, `Name` |
| [`Stc.Syntax.Cst`](../../lean/Stc/Syntax/Cst.lean) | `NodeKind`, `Cst`, `Cst.leaves`, `CoversFrom`, `Lossless`, `Cst.WellFormedNodes`, `Cst.WellFormed`, `Cst.size` |
| [`Stc.Syntax.Ast`](../../lean/Stc/Syntax/Ast.lean) | `TypeSpec`, `Literal`, `Expr`, `Place`, `Stmt`, `StmtList`, `VarDecl`, `VarSection`, `FunctionDecl`, `ProgramDecl`, `TopDecl`, `SourceUnit`, `Ast`, `Ast.SpansNested`, `ast_span_wellformed` |
| [`Stc.Grammar.Derives`](../../lean/Stc/Grammar/Derives.lean) | `Derives`, the list/optional helpers, `MapsTo`, `MapsToWithSpan`, `Derives.unit_span_eq` |
| [`Stc.Grammar.Examples`](../../lean/Stc/Grammar/Examples.lean) | A checked `PROGRAM` derivation exercising declarations, statements, and the expression chain |

The umbrella modules are [`Stc.Syntax`](../../lean/Stc/Syntax.lean) and
[`Stc.Grammar`](../../lean/Stc/Grammar.lean).

## 2. Token model

### 2.1 Spans and source coverage

| Lean definition | Meaning | Specification |
| --- | --- | --- |
| `FileId` | File index for a span | STC-LEX-006, STC-LEX-007 |
| `Span` | Byte range `[start, stop)` in a file | STC-LEX-005, STC-LEX-006 |
| `Span.WellFormed` | `start ≤ stop` | STC-LEX-006 |
| `Span.contains` | Same file and inner range inside the outer range | STC-LEX-006 |
| `Span.union` | Least span covering two spans (same file) | Span composition |
| `Cst.leaves` | Token and trivia spans in source order | STC-LEX-004, STC-LEX-011 |
| `CoversFrom` | Contiguous, ordered, gap-free, overlap-free coverage | STC-LEX-004, STC-LEX-011 |
| `Lossless` | `CoversFrom 0 sourceLength` over `Cst.leaves` | STC-LEX-004 |

The model records byte offsets. Line/column conversion (`SourceMap`) and UTF-8 validation remain
Rust responsibilities; P0.1 does not re-model them. The lossless contract is stated over the
token/trivia sequence, not over a concrete rowan tree.

### 2.2 Token and trivia kinds

| Lean definition | Covered spellings / values | Specification |
| --- | --- | --- |
| `TokenKind.ident` | Identifier with original spelling; `Name.key` is ASCII uppercase | STC-LEX-008, STC-LEX-009, STC-LEX-012–014 |
| `TokenKind.keyword` | The core keyword enum in `Keyword` | Grammar §3; Tokenization §3 |
| `TokenKind.intLit` | Decimal integer literal value | STC-LEX-017, STC-LEX-019, STC-LEX-023 |
| `TokenKind.basedLit` | `2#`, `8#`, `16#` bit pattern | STC-LEX-020, STC-LEX-021 |
| `TokenKind.typedInt` | `<elementary numeric type>#<numeric literal>` | STC-LEX-022 |
| `TokenKind.typedBool` | `BOOL#TRUE`, `BOOL#FALSE`, `BOOL#0`, `BOOL#1` | STC-LEX-024 |
| `TokenKind.charLit` | `CHAR#`, `WCHAR#`, `UCHAR#` with a decoded scalar | STC-LEX-030, STC-LEX-035 |
| `TokenKind.punct` | The core punctuation enum in `Punct` | Tokenization §9 |
| `TokenKind.eof` | End-of-file marker | Tokenization §2.2 |
| `TokenKind.error` | Recovery token; never accepted by the grammar | Tokenization §10.2 |
| `TriviaKind.whitespace` | Horizontal whitespace and line terminators | STC-LEX-010, STC-LEX-047 |
| `TriviaKind.lineComment` / `blockComment` / `docComment` | `//`, `(* *)`, `/* */`, `///` | STC-LEX-047–051 |
| `TriviaKind.bom` | Initial UTF-8 BOM | STC-LEX-001, STC-LEX-003 |

Excluded token classes: `RealLiteral`, `StringLiteral`, `UStringLiteral`, `DurationLiteral`,
`DateLiteral`, `TodLiteral`, `DateTimeLiteral`, `DirectVariable`, and `Pragma`. They are outside
the frozen core and are rejected by the reference tools rather than modeled (charter §6).

### 2.3 Names and elementary types

| Lean definition | Meaning |
| --- | --- |
| `Name` | Identifier spelling plus span; equality and lookup use `Name.key` |
| `Name.ofToken?` | Recover a name from an `ident` token; `ofToken?_span` proves the span is preserved |
| `ElementaryType` | `BOOL`, signed and unsigned integers, bit strings, `CHAR`, `WCHAR`, `UCHAR` |
| `CharType` | Character layout (`char`, `wchar`, `uchar`) |
| `ElementaryType.ofKeyword?` | Maps a core type keyword to its elementary type |
| `TypeSpec.ofToken?` | Recovers a `TypeSpec` from a type keyword token; `ofToken?_span` preserves the span |

Widths, signedness, and conversion behavior are defined in P0.2 against `Stc.Exact`; P0.1 fixes
only the surface names.

## 3. CST contract

`Cst` is a contract-level tree:

```text
Cst ::= token Token | trivia Trivia | node NodeKind Span (List Cst)
```

`NodeKind` names every production the core grammar relation consumes (source file, top item, var
section, var declaration, function/program declarations, statement kinds, expression precedence
levels, places, calls, arguments, case elements). Trivia is retained in the tree so the lossless
contract is meaningful; `Cst.stripTrivia` produces the significant projection that `Derives`
matches.

The contract is checked by predicates, not by a rowan implementation:

| Predicate | Statement |
| --- | --- |
| `Lossless c file sourceLength` | `c.leaves` covers `[0, sourceLength)` in order with no gaps or overlaps |
| `Cst.WellFormedNodes c` | Every node span is well formed and contains its direct children's spans |
| `Cst.NodeSpansWithin c file sourceLength` | Every node span is in the source file and within its bounds |
| `Cst.WellFormed c file sourceLength` | `Lossless` plus `WellFormedNodes` plus `NodeSpansWithin` |

Basic lemmas:

| Lemma | Statement |
| --- | --- |
| `CoversFrom.pos_le_stop` | A covering sequence does not extend past its end bound |
| `CoversFrom.mem_bounds` | Every covered span lies inside the bounds and is well formed |
| `Lossless.leaf_bounds` | Every leaf of a lossless CST lies inside the source |
| `Cst.WellFormedNodes.contains_child` | A node span contains each direct child span |
| `Cst.WellFormedNodes.child` | Every direct child is well formed |
| `Cst.WellFormedNodes.contains_significantChild` | A node span contains each significant child span |
| `Cst.stripTrivia_span` | Removing trivia preserves the item's span |
| `Cst.WellFormed.lossless` / `.nodes` / `.node_bounds` / `.span_bounds` | Components of the full contract |
| `Cst.size_child_lt` | A direct child is strictly smaller than its parent |

## 4. Surface AST

The AST is the inductive family in `Stc.Syntax.Ast`:

| Area | Lean definitions |
| --- | --- |
| Types and literals | `TypeSpec`, `ElementaryType`, `CharType`, `Literal`, `LiteralKind`, `Literal.ofToken?` |
| Places and expressions | `Place`, `Expr`, `UnaryOp`, `BinaryOp`, `CallArg` |
| Statements | `Stmt`, `StmtList`, `IfArm`, `CaseArm`, `CaseLabel` |
| Declarations and programs | `VarDecl`, `VarKind`, `VarSection`, `FunctionDecl`, `ProgramDecl`, `TopDecl`, `SourceUnit` |
| Lists and views | `ArgumentList`, `CaseLabelList`, `Ast`, `Ast.span`, `Ast.children`, `Ast.Descendant`, `Ast.SpansNested` |

`Expr.literal` and `Expr.var` carry an explicit span; `Expr.withSpan` replaces the root span when a
precedence production wraps a lower-level expression. Every AST node therefore has a span, and
`Ast.children` exposes the immediate children used by the span invariant.

## 5. Grammar relation coverage

`Derives : Cst → Ast → Prop` is the declarative grammar relation. Each row below maps an EBNF
production to its Lean rule(s). `Derives` consumes the trivia-free projection; `MapsTo` is the
full-CST wrapper.

### 5.1 Source unit and declarations

| Rule ID | Production | Lean rule(s) | Notes |
| --- | --- | --- | --- |
| STC-GRAM-001 | `source_file` | `Derives.sourceFile` | `top_item* EOF`; EOF is a required token |
| STC-GRAM-001 | `top_item` | `Derives.topItemGlobal`, `topItemFunction`, `topItemProgram` | No pragmas; no namespace, data-type, FB, class, interface, or configuration alternatives |
| STC-GRAM-001 | `pragmas` | — | The core relation requires zero pragmas; pragma tokens and their attachments are excluded |
| STC-GRAM-005 | `function_decl` | `Derives.functionDecl` | Optional result type; no `USING` |
| STC-GRAM-005 | `program_decl` | `Derives.programDecl` | No `VAR_ACCESS` |
| STC-GRAM-004 | `var_section` | `Derives.varSection` | `VAR`, `VAR_INPUT`, `VAR_GLOBAL`; no qualifiers or `AT` |
| STC-GRAM-004 | `global_var_section` | `Derives.varSection` + `Derives.topItemGlobal` | The section kind must be `VAR_GLOBAL` at top level |
| STC-GRAM-004 | `var_kind` | `Keyword.varKind?` with `VarKind.var`, `.input`, `.global` | `VAR_OUTPUT`, `VAR_IN_OUT`, `VAR_TEMP`, `VAR_EXTERNAL` excluded |
| STC-GRAM-004 | `var_decl` | `Derives.varDecl` | `IDENT (',' IDENT)* ':' type (':=' expression)? ';'` |
| STC-GRAM-003 | `type_spec` | `Derives.typeSpec` | Elementary type names only |
| STC-GRAM-004 | `var_type` | `Derives.typeSpec` | Core `var_type` is `type_spec`; array, reference, pointer, and qualified alternatives are excluded |
| Grammar §10 | `elementary_type_name` and its sub-productions | `ElementaryType.ofKeyword?` | `numeric_type_name`, `int_type_name`, `signed_int_type_name`, `unsigned_int_type_name`, `bit_type_name`, `string_type_name` are covered by the keyword classification; only core names are accepted |
| STC-GRAM-003 | `initializer` | `Derives.varDecl` (expression child) | Scalar `initializer ::= expression`; array/struct initializers excluded |

`function_decl` and `program_decl` accept any core `var_section` in their section lists at the
surface layer. Placement rules (`VAR_GLOBAL` only at top level, function `VAR_INPUT` only, and so
on) are checked by typing in P0.2, not by the grammar relation.

### 5.2 Statements

| Rule ID | Production | Lean rule(s) | Notes |
| --- | --- | --- | --- |
| STC-GRAM-007 / STC-GRAM-016 | `stmt_list` | `StatementListRel` + `Derives.statementList` | Strict semicolons; an omitted statement is `Stmt.empty` |
| STC-GRAM-007 | `stmt` | `Stmt` constructors | Core alternatives only |
| STC-GRAM-007 | `assignment_stmt` | `Derives.assignmentStatement` | Place is a simple variable |
| STC-GRAM-007 | `call_stmt` | `Derives.callStatement` | Call expression used as a statement |
| STC-GRAM-007 | `return_stmt` | `Derives.returnStatement` | |
| STC-GRAM-007 | `exit_stmt` | `Derives.exitStatement` | |
| STC-GRAM-007 | `continue_stmt` | `Derives.continueStatement` | |
| STC-GRAM-007 | `if_stmt` | `Derives.ifStatement`, `elsifClause`, `elseClause` | `ELSIF*` and optional `ELSE` |
| STC-GRAM-007 | `case_stmt` | `Derives.caseStatement` | `case_selection+` enforced by `ListRel1` |
| STC-GRAM-007 | `case_selection` | `Derives.caseSelection` | `case_list ':' stmt_list` |
| STC-GRAM-007 | `case_list` | `Derives.caseList` | Comma-separated elements |
| STC-GRAM-007 | `case_element` | `Derives.caseElementSingle`, `caseElementRange` | Integer constant/range; constant restriction is P0.2 |
| STC-GRAM-007 | `constant_expr` | `Derives.constantExpression` | The production is represented; the constant-only restriction is P0.2 |
| STC-GRAM-007 | `for_stmt` | `Derives.forStatement` | Optional `BY` |
| STC-GRAM-007 | `while_stmt` | `Derives.whileStatement` | |
| STC-GRAM-007 | `repeat_stmt` | `Derives.repeatStatement` | |
| STC-GRAM-007 | `ref_assignment`, `assignment_attempt` | — | Excluded (`REF_TO`, pointers, and attempts are outside the core) |

### 5.3 Places and expressions

| Rule ID | Production | Lean rule(s) | Notes |
| --- | --- | --- | --- |
| Grammar §9.4 | `place` | `Derives.place` | Wraps a core variable |
| Grammar §9.4 | `variable` | `Derives.variable` | Simple path; no `THIS`, postfix, or subscript |
| Grammar §9.4 | `path` | `Derives.path` | One identifier segment; qualified paths excluded |
| STC-GRAM-008 | `expression` | `Derives.expression` | `or_expr` entry |
| STC-GRAM-008/009 | `or_expr` | `Derives.orExpression` | Left-associative chain via `DerivesChained` |
| STC-GRAM-008 | `xor_expr` | `Derives.xorExpression` | |
| STC-GRAM-008/012 | `and_expr` | `Derives.andExpression` | Accepts `AND` and `&` |
| STC-GRAM-008/009 | `compare_expr` | `Derives.compareExpression` | Equality `=` / `<>` (level 4) |
| STC-GRAM-008/009 | `equ_expr` | `Derives.equExpression` | Relational `<` `>` `<=` `>=` (level 5) |
| STC-GRAM-008 | `add_expr` | `Derives.addExpression` | |
| STC-GRAM-008 | `term` | `Derives.term` | `*`, `/`, `MOD` |
| STC-GRAM-008/010/011 | `power_expr` | `Derives.powerExpression` | `**`, left-associative; unary binds tighter |
| STC-GRAM-008/015 | `unary_expr` | `Derives.unaryOperator`, `unaryOperand` | `-`, `+`, `NOT` |
| STC-GRAM-008 | `postfix_expr` | `Derives.postfixExpression` | `postfix*` is empty in the core |
| STC-GRAM-008 | `primary_expr` | `Derives.primaryLiteral`, `primaryVariableOrCall`, `primaryParen` | No enum values, references, `ADR`, `SIZEOF`, pointer conversions |
| STC-GRAM-008 | `paren_expr` | `Derives.parenExpression` | `( expression )` |
| STC-GRAM-008 | `literal` | `Derives.literalExpression` + `Literal.ofToken?` | Integer, based, typed integer, typed boolean, character, `TRUE`, `FALSE` |
| STC-GRAM-008 | `variable_or_call` | `Derives.variableOrCallVariable`, `variableOrCallCallEmpty`, `variableOrCallCallArgs` | Simple path; `F` vs `F()` distinguished |
| STC-GRAM-008 | `call_expression` | `Derives.callExpression` | Wrapper used by `call_stmt` |
| STC-GRAM-008 | `arg_list` | `Derives.argumentList` | Non-empty; comma-separated |
| STC-GRAM-008 | `arg` | `Derives.argumentPositional`, `argumentNamed` | Positional and named input arguments; output bindings excluded |
| Grammar §9.2 | `ref_value`, `address_of`, `sizeof_expr`, `pointer_conversion` | — | Excluded |

The precedence order in the Lean rules mirrors the production nesting: `or_expr` → `xor_expr` →
`and_expr` → `compare_expr` (equality) → `equ_expr` (relational) → `add_expr` → `term` →
`power_expr` → `unary_expr` → `postfix_expr` → `primary_expr`. This implements STC-GRAM-009
(relational tighter than equality), STC-GRAM-010 (`**` left-associative), STC-GRAM-011 (unary
tighter than `**`), STC-GRAM-012 (`&`/`AND` and `=` as equality), and STC-GRAM-015 (`NOT`
precedence). STC-GRAM-013 (short-circuiting) and STC-GRAM-014 (evaluation order) are dynamic
semantics owned by P0.3; the grammar relation preserves the operand order.

### 5.4 Excluded productions

The following Grammar productions have no `Derives` rule and are rejected by the reference tools:
data types and all type declarations; namespaces and `USING`; configuration, resource, and task
declarations; function blocks, classes, interfaces, methods, `THIS`/`SUPER`; references, pointers,
`ADR`, `SIZEOF`, pointer conversions; arrays, structs, enums, subranges, strings, time/date types;
`VAR_OUTPUT`, `VAR_IN_OUT`, `VAR_TEMP`, `VAR_EXTERNAL`, qualifiers, `AT`, edge declarations;
`VAR_ACCESS`, `VAR_CONFIG`; pragmas; `EXIT`/`CONTINUE` are included, but SFC actions/steps are not;
IL/LD/FBD bodies; preprocessor directives. The exclusion register and the owning later phase are
in [Semantics-Charter.md](Semantics-Charter.md#6-frozen-core-and-exclusion-register).

## 6. AST-to-CST and source-span mapping

| Definition / theorem | Statement |
| --- | --- |
| `Derives c a` | The trivia-free CST `c` grammatically derives the AST node `a` |
| `MapsTo c a` | `∃ c', Cst.stripTrivia c = some c' ∧ Derives c' a` |
| `MapsToWithSpan c a` | `MapsTo c a ∧ a.span = c.span` |
| `Derives.unit_span_eq` | A derived `SourceUnit` carries the `sourceFile` node's span |
| `Derives.unit_span_wellFormed` | A derived unit's span is well formed when its CST is |
| `Derives.unit_span_bounded` | A derived unit's span lies within the declared source file and length |
| `ast_span_wellformed` | For a span-nested node, every descendant is span-nested and contained in the root |
| `Ast.SpansNested` | Every node span is well formed and contains its children's spans |

`ast_span_wellformed` is proved with no axioms and is registered as `P0-T01`. The per-node
`Derives`-to-CST containment lemma (every derived node's span is contained in its deriving CST
node's span) is *not* a P0.1 exit requirement and is not proved here; Phase 4's parser-soundness
proof establishes it from `Cst.WellFormed` and the rule shapes. The P0.1 relation already fixes
the spans and the mapping predicates the proof will use.

## 7. Validation and limits

- [`Stc.Grammar.Examples`](../../lean/Stc/Grammar/Examples.lean) contains a checked derivation of
  a complete `PROGRAM` with a scalar global/local declaration, an assignment, and a literal
  expression through every precedence level. It is a smoke test of the rule shapes, not a parser
  test.
- The token model is a proof surface, not a lexer implementation. Lexer rule conformance remains
  tested in Rust; P0.1 does not prove the lexer.
- The CST is a contract, not a rowan tree. The full-CST lossless predicate and the significant
  projection are the interface Phase 4 consumes.
- No parser correctness claim is made. `Derives` is a declarative relation; it is not executable
  and does not by itself show that a parser produces a derivation.

## 8. Review status

P0.1 coverage was reviewed and approved on 2026-10-27 against
[Grammar](../Syntax/Grammar.md) and [Tokenization](../Syntax/Tokenization.md). Every included core
production has a Lean constructor or relation rule; every excluded production is listed in §5.4
and the charter register. The approval is recorded in [Plan-Phase0.md](Plan-Phase0.md) §17.
