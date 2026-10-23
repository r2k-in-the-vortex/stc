# Structured Text Grammar

Status: Draft  
Profile: `stc-2013-oop-utf8-v1`  
Rule IDs: `STC-GRAM-*`  
Related: [Tokenization](Tokenization.md), [AST](../AST/AST.md), [Conformance](../01-Conformance-Profile.md)

This is the accepted surface grammar for `stc`. It is a cleaned-up, ST-only form of the 2013
grammar: IL, LD, FBD, and SFC bodies are excluded; classes, interfaces, inheritance, methods,
namespaces, references, and configuration declarations are included; the UTF-8 extension is
marked `STC-UTF8`.

## 1. Notation

- Terminals are written in single quotes. Keywords and punctuation are **case-insensitive** for
  keywords; punctuation is literal.
- `IDENT`, `INT`, `REAL`, `STRING`, `USTRING`, `PRAGMA`, and `DIRECT_VAR` are token classes
  defined in [Tokenization](Tokenization.md). Grammar shorthand: `INT` = `IntLiteral`,
  `REAL` = `RealLiteral`, `STRING` = `StringLiteral`, `USTRING` = `UStringLiteral`,
  `CHAR_LIT` = `CharLiteral`, `DURATION` = `DurationLiteral`, `DATE` = `DateLiteral`,
  `TOD` = `TodLiteral`, `DT` = `DateTimeLiteral`, `DIRECT_VAR` = `DirectVariable`.
- `?` = optional, `*` = zero or more, `+` = one or more, `(a | b)` = alternation.
- `(* ... *)` is a grammar comment, not ST source.
- A production name may be followed by a rule ID in a comment.
- `pragmas` may precede any top-level item, POU, method, or variable declaration. Pragmas are
  attached to the following construct.

## 2. Source unit

```ebnf
(* STC-GRAM-001 *)
source_file       ::= top_item* EOF ;

pragmas           ::= PRAGMA* ;

top_item          ::= pragmas ( using_directive
                              | namespace_decl
                              | data_type_decl
                              | function_decl
                              | function_block_decl
                              | program_decl
                              | class_decl
                              | interface_decl
                              | configuration_decl
                              | global_var_section ) ;
```

A file may contain multiple POUs and a configuration. The compiler builds one project from all
source files listed by `stc.toml`; file order is not semantically significant.

## 3. Names, namespaces, and USING

```ebnf
(* STC-GRAM-002 *)
qualified_name     ::= IDENT ( '.' IDENT )* ;
namespace_name     ::= IDENT ( '.' IDENT )* ;

using_directive    ::= 'USING' namespace_name ( ',' namespace_name )* ';' ;

namespace_decl     ::= 'NAMESPACE' 'INTERNAL'? namespace_name
                       using_directive*
                       namespace_element+
                       'END_NAMESPACE' ;

namespace_element  ::= data_type_decl
                     | function_decl
                     | function_block_decl
                     | class_decl
                     | interface_decl
                     | namespace_decl ;
```

- Namespaces may nest. A namespace element may not be a `PROGRAM`, `CONFIGURATION`, or global
  variable section; those are top-level only (2013 namespace rules).
- `USING` imports all public names of the named namespace into the current scope. Resolution
  rules and shadowing are in the AST/analyzer spec.
- `INTERNAL` limits visibility to the declaring namespace subtree and its package (see the project
  model); `PRIVATE` is POU/class-private.
- Package boundaries are not syntax. A dependency's package name (or manifest alias) is exposed
  as a root namespace; cross-package references use qualified names such as
  `motor-lib.Main.Motor`. A package's top-level declarations are public across packages unless
  `INTERNAL`.

## 4. Data types

```ebnf
(* STC-GRAM-003 *)
data_type_decl     ::= 'TYPE' ( type_decl ';' )+ 'END_TYPE' ;

type_decl          ::= simple_type_decl
                     | subrange_type_decl
                     | enum_type_decl
                     | named_value_type_decl
                     | array_type_decl
                     | struct_type_decl
                     | string_type_decl
                     | ref_type_decl ;

simple_type_decl   ::= IDENT ':' type_spec ( ':=' constant_expr )? ;

subrange_type_decl ::= IDENT ':' int_type_name
                       '(' constant_expr '..' constant_expr ')'
                       ( ':=' signed_int )? ;

enum_type_decl     ::= IDENT ':' '(' enum_value ( ',' enum_value )* ')'
                       ( ':=' enum_value )? ;

enum_value         ::= IDENT ( ':=' INT )? ;

named_value_type_decl ::= IDENT ':' elementary_type_name
                          '(' named_value ( ',' named_value )* ')'
                          ( ':=' named_value )? ;

named_value        ::= IDENT ( ':=' constant_expr )? ;

array_type_decl    ::= IDENT ':' array_type ( ':=' array_init )? ;

struct_type_decl   ::= IDENT ':' 'STRUCT' 'OVERLAP'?
                       ( struct_elem ';' )+
                       'END_STRUCT'
                       ( ':=' struct_init )? ;

struct_elem        ::= IDENT ( 'AT' DIRECT_VAR )? ':' type_spec
                       ( ':=' initializer )? ;

string_type_decl   ::= IDENT ':' string_type_name ( ':=' char_str )? ;

ref_type_decl      ::= IDENT ':' 'REF_TO' type_spec ( ':=' ref_value )? ;

type_spec          ::= elementary_type_name
                     | string_type_name
                     | 'ARRAY' '[' array_bound ( ',' array_bound )* ']' 'OF' type_spec
                     | 'STRUCT' 'OVERLAP'? ( struct_elem ';' )+ 'END_STRUCT'
                     | 'REF_TO' type_spec
                     | qualified_name ;

array_type         ::= 'ARRAY' '[' array_bound ( ',' array_bound )* ']' 'OF' type_spec ;

array_bound        ::= constant_expr
                     | constant_expr '..' constant_expr
                     | '*' ;
```

Notes:

- `STRUCT ... END_STRUCT` may be used inline in a variable declaration or named in `TYPE`.
- `OVERLAP` requests overlapping storage for `AT`-addressed struct elements. The analyzer checks
  that the overlapping fields have compatible bit ranges.
- `ARRAY [*]` is only valid in a conformant context (`VAR_IN_OUT` or an FB input/output that is
  declared with `ARRAY [*]`), never for a stored global, local, or function result. v1 implements
  conformant parameters with a descriptor pointing at caller-owned storage (A-007).
- A `REF_TO` type may not be `REF_TO` of another reference directly; use a named reference type
  if needed. This is a semantic restriction, recorded as `STC-TYPE-*`.
- `STRING`, `WSTRING`, `CHAR`, `WCHAR` are 2013 types. `USTRING` and `UCHAR` are `STC-UTF8`
  extensions.
- `named_value_type_decl` covers the 2013 "data type with named values" form (for example a
  `DWORD` with `Red := 16#00FF0000`).

## 5. Variable declarations

```ebnf
(* STC-GRAM-004 *)
var_section        ::= var_kind var_qualifier* ( var_decl | edge_decl )* 'END_VAR' ;

var_kind           ::= 'VAR'
                     | 'VAR_INPUT'
                     | 'VAR_OUTPUT'
                     | 'VAR_IN_OUT'
                     | 'VAR_TEMP'
                     | 'VAR_EXTERNAL'
                     | 'VAR_GLOBAL' ;

var_qualifier      ::= 'CONSTANT'
                     | 'RETAIN'
                     | 'NON_RETAIN'
                     | access_spec ;

global_var_section ::= var_section ;  (* kind must be VAR_GLOBAL *)

var_decl           ::= pragmas
                       IDENT ( ',' IDENT )*
                       ( 'AT' DIRECT_VAR )?
                       ':' var_type
                       ( ':=' initializer )?
                       ';' ;

var_type           ::= type_spec
                     | 'ARRAY' '[' '*' ( ',' '*' )* ']' 'OF' type_spec
                     | 'REF_TO' type_spec
                     | qualified_name ;  (* class, interface, FB, or POU type *)

edge_decl          ::= pragmas
                       IDENT ( ',' IDENT )*
                       ':' 'BOOL' ( 'R_EDGE' | 'F_EDGE' ) ';' ;

access_decl        ::= IDENT ':' access_path ':' type_spec
                       ( 'READ_ONLY' | 'READ_WRITE' )? ';' ;

access_path        ::= ( IDENT '.' )? DIRECT_VAR
                     | ( IDENT '.' )? ( IDENT '.' )* qualified_name
                     | qualified_name ;
```

Rules and notes:

- `var_qualifier` combinations are checked semantically. `CONSTANT` with `VAR_IN_OUT` is invalid;
  `RETAIN`/`NON_RETAIN` are meaningful only on selected variable sections.
- `AT` binds a variable or struct element to a directly represented address. `DIRECT_VAR` may be
  a wildcard (`%I*`, `%Q*`, `%M*`) in declarations and `VAR_ACCESS`.
- `edge_decl` is an alternative form for `BOOL` edge variables; it is written as a normal
  `var_decl` with the `R_EDGE`/`F_EDGE` suffix and no initializer.
- `VAR_EXTERNAL` declares names owned by a global variable section; the analyzer links them.
- `VAR_ACCESS` is its own section (`var_access_section`), not a `var_kind`; it may appear at
  program/configuration level. The access path syntax is deliberately broad; binding is checked
  by the analyzer.
- `VAR_TEMP` variables are not retained across cycles.
- `VAR_CONFIG` is only valid inside a `CONFIGURATION` and has its own production
  (`config_init`), not `var_section`; see §7.

## 6. Program organization units

```ebnf
(* STC-GRAM-005 *)
function_decl      ::= 'FUNCTION' IDENT ( ':' type_spec )?
                       using_directive*
                       var_section*
                       stmt_list
                       'END_FUNCTION' ;

function_block_decl ::= 'FUNCTION_BLOCK'
                        ( 'FINAL' | 'ABSTRACT' )?
                        IDENT
                        using_directive*
                        ( 'EXTENDS' ( qualified_name ) )?
                        ( 'IMPLEMENTS' interface_list )?
                        var_section*
                        method_decl*
                        stmt_list?
                        'END_FUNCTION_BLOCK' ;

program_decl       ::= 'PROGRAM' IDENT
                       using_directive*
                       var_section*
                       var_access_section?
                       stmt_list
                       'END_PROGRAM' ;

class_decl         ::= 'CLASS'
                       ( 'FINAL' | 'ABSTRACT' )?
                       IDENT
                       using_directive*
                       ( 'EXTENDS' qualified_name )?
                       ( 'IMPLEMENTS' interface_list )?
                       class_var_section*
                       method_decl*
                       'END_CLASS' ;

class_var_section  ::= 'VAR' var_qualifier* var_decl* 'END_VAR' ;
                       (* only VAR; INPUT/OUTPUT/IN_OUT/TEMP are illegal *)

interface_decl     ::= 'INTERFACE' IDENT
                       using_directive*
                       ( 'EXTENDS' interface_list )?
                       method_prototype*
                       'END_INTERFACE' ;

method_decl        ::= 'METHOD'
                       access_spec?
                       ( 'FINAL' | 'ABSTRACT' )?
                       'OVERRIDE'?
                       IDENT
                       ( ':' type_spec )?
                       using_directive*
                       var_section*
                       stmt_list?
                       'END_METHOD' ;

method_prototype   ::= 'METHOD'
                       access_spec?
                       IDENT
                       ( ':' type_spec )?
                       using_directive*
                       var_section*
                       'END_METHOD' ;

interface_list     ::= qualified_name ( ',' qualified_name )* ;

access_spec        ::= 'PUBLIC' | 'PROTECTED' | 'PRIVATE' | 'INTERNAL' ;
```

Rules and notes:

- `FUNCTION` bodies are statement lists. Functions may have no result type; a function with a
  result must assign to its own name on every path that returns a value.
- `FUNCTION_BLOCK` may have no body (methods only), matching the 2013 OOP extension. `SUPER()` in
  a derived FB body calls the base FB body.
- `CLASS` has no body and only `VAR` sections. Class instances are values; interfaces hold
  references to class/FB instances.
- `EXTENDS` on an FB may name an FB type or a class; on a class it names a class; on an interface
  it names one or more interfaces.
- `IMPLEMENTS` takes one or more interfaces. All interface methods must be implemented unless the
  class/FB is `ABSTRACT`.
- `method_decl` appears in classes, FBs, and (as `method_prototype`) interfaces. `OVERRIDE` is
  only valid when the method overrides a base method; `ABSTRACT` is only valid in an abstract
  class/FB, and is incompatible with `OVERRIDE`.
- Default access: variables in classes/FBs are `PROTECTED`; methods are `PUBLIC` unless marked.
  This is a `stc` choice; the 2013 tables describe the allowed access specifiers but leave some
  default details to the implementer.
- `THIS` and `SUPER` are available in method/FB bodies. `THIS` is not a general expression
  prefix on arbitrary instances.
- `PRAGMA`s may precede a POU or method; `{stc:test}` and `{stc:export}` are recognized.

## 7. Configurations, resources, and tasks

```ebnf
(* STC-GRAM-006 *)
configuration_decl ::= 'CONFIGURATION' IDENT
                       global_var_section?
                       ( resource_decl+ | single_resource_decl )
                       var_access_section?
                       config_init?
                       'END_CONFIGURATION' ;

resource_decl      ::= 'RESOURCE' IDENT 'ON' IDENT
                       global_var_section?
                       single_resource_decl
                       'END_RESOURCE' ;

single_resource_decl ::= ( task_config ';' )*
                         ( program_config ';' )+ ;

task_config        ::= 'TASK' IDENT '(' task_attr ( ',' task_attr )* ')' ;

task_attr          ::= ( 'SINGLE' ':=' data_source )
                     | ( 'INTERVAL' ':=' data_source )
                     | ( 'PRIORITY' ':=' INT ) ;

program_config     ::= 'PROGRAM' ( 'RETAIN' | 'NON_RETAIN' )?
                       IDENT ( 'WITH' IDENT )?
                       ':' qualified_name
                       ( '(' prog_conf_elem ( ',' prog_conf_elem )* ')' )? ;

prog_conf_elem     ::= fb_task | prog_cnxn ;

fb_task            ::= IDENT 'WITH' IDENT ;

prog_cnxn          ::= variable ':=' prog_data_source
                     | variable '=>' data_sink ;

prog_data_source   ::= constant_expr | enum_value | variable | DIRECT_VAR ;

data_sink          ::= variable | DIRECT_VAR ;

var_access_section ::= 'VAR_ACCESS' access_decl* 'END_VAR' ;

config_init        ::= 'VAR_CONFIG' ( config_inst_init ';' )* 'END_VAR' ;

config_inst_init   ::= IDENT '.' IDENT '.'
                       ( IDENT '.' )*
                       config_target ;

config_target      ::= IDENT ( 'AT' DIRECT_VAR )? ':' var_type
                       ( ':=' initializer )?
                     | ( IDENT ':' qualified_name ) ':=' struct_init ;
```

Notes:

- The configuration model is parsed and analyzed even when the runtime implements only one task
  in v1. Multiple tasks, priorities, and intervals are accepted syntactically; the runtime
  reports `STC-E-RUN-001` when the selected execution mode cannot honor them.
- `SINGLE`/`INTERVAL` data sources are constants, global access paths, program outputs, or direct
  variables.
- Task interval units are durations (`T#...`).
- `VAR_ACCESS` declarations describe externally visible access paths. Runtime binding is staged;
  the resolver validates paths and types.

## 8. Statements

```ebnf
(* STC-GRAM-007 *)
stmt_list          ::= ( stmt? ';' )* ;

stmt               ::= assignment_stmt
                     | ref_assignment
                     | assignment_attempt
                     | call_stmt
                     | return_stmt
                     | if_stmt
                     | case_stmt
                     | for_stmt
                     | while_stmt
                     | repeat_stmt
                     | exit_stmt
                     | continue_stmt ;

assignment_stmt    ::= variable ':=' expression ;

ref_assignment     ::= ref_name ':=' ( ref_name | ref_deref | ref_value ) ;

assignment_attempt ::= ( ref_name | ref_deref ) '?='
                       ( ref_name | ref_deref | ref_value ) ;

call_stmt          ::= call_expression ;

return_stmt        ::= 'RETURN' ;

exit_stmt          ::= 'EXIT' ;
continue_stmt      ::= 'CONTINUE' ;

if_stmt            ::= 'IF' expression 'THEN' stmt_list
                       ( 'ELSIF' expression 'THEN' stmt_list )*
                       ( 'ELSE' stmt_list )?
                       'END_IF' ;

case_stmt          ::= 'CASE' expression 'OF'
                       case_selection+
                       ( 'ELSE' stmt_list )?
                       'END_CASE' ;

case_selection     ::= case_list ':' stmt_list ;

case_list          ::= case_element ( ',' case_element )* ;

case_element       ::= constant_expr ( '..' constant_expr )? ;

for_stmt           ::= 'FOR' IDENT ':=' expression 'TO' expression
                       ( 'BY' expression )?
                       'DO' stmt_list 'END_FOR' ;

while_stmt         ::= 'WHILE' expression 'DO' stmt_list 'END_WHILE' ;

repeat_stmt        ::= 'REPEAT' stmt_list 'UNTIL' expression 'END_REPEAT' ;

ref_name           ::= IDENT ;
ref_deref          ::= ref_name '^'+ ;
```

Rules and notes:

- A `call_stmt` is syntactically any call expression. The analyzer decides whether it is a
  function call (result may be discarded), an FB invocation, or a method call. A function call
  used as a statement is allowed; its result is discarded.
- `RETURN` exits a function, method, FB body, or program early. It may be followed by `;`.
- `FOR` uses `BY`, not `STEP`. The 2013 prose contains an example with `STEP`; the normative
  grammar uses `BY`, and `stc` follows the grammar.
- The `FOR` control variable is a simple identifier and must not be modified inside the loop.
- `CASE` labels are constants, enum values, or subranges. The first matching selection wins;
  `ELSE` is optional.
- `EXIT` exits the innermost `FOR`/`WHILE`/`REPEAT`. `CONTINUE` jumps to the loop's next
  iteration point. Both are supported for all three loop forms.
- An empty statement is written as `;` and is allowed by `stmt_list`.
- Semicolon relaxation: `stc` accepts an omitted semicolon after a block statement (`IF`, `CASE`,
  `FOR`, `WHILE`, `REPEAT`) when the next token is a block delimiter (`ELSE`, `ELSIF`, `END_*`,
  `UNTIL`) or another statement in a nested block. Simple statements still require `;`.
  This accommodation matches common 2013-era ST source; it is a syntax superset and is tested.

## 9. Expressions

```ebnf
(* STC-GRAM-008 *)
expression         ::= or_expr ;

or_expr            ::= xor_expr ( 'OR' xor_expr )* ;

xor_expr           ::= and_expr ( 'XOR' and_expr )* ;

and_expr           ::= compare_expr ( ( '&' | 'AND' ) compare_expr )* ;

compare_expr       ::= add_expr ( compare_op add_expr )* ;

compare_op         ::= '=' | '<>' | '<' | '>' | '<=' | '>=' ;

add_expr           ::= term ( ( '+' | '-' ) term )* ;

term               ::= power_expr ( ( '*' | '/' | 'MOD' ) power_expr )* ;

power_expr         ::= unary_expr ( '**' unary_expr )* ;

unary_expr         ::= ( '-' | '+' | 'NOT' ) unary_expr
                     | postfix_expr ;

postfix_expr       ::= primary_expr postfix* ;

postfix            ::= '.' IDENT
                     | '.' INT
                     | '.' '%' ( 'X' | 'B' | 'W' | 'D' | 'L' )? INT
                     | '[' expression ( ',' expression )* ']'
                     | '^' ;

primary_expr       ::= literal
                     | enum_value
                     | variable_or_call
                     | ref_value
                     | '(' expression ')' ;

variable_or_call   ::= ( 'THIS' | 'SUPER' ) ( '.' path )? ( '(' arg_list? ')' )?
                     | path ( '(' arg_list? ')' )?
                     | 'SUPER' '(' ')' ;

call_expression    ::= variable_or_call ;

path               ::= IDENT ( '.' IDENT )* ;

arg_list           ::= arg ( ',' arg )* ;

arg                ::= ( IDENT ':=' )? expression
                     | ref_assignment
                     | 'NOT'? IDENT '=>' variable ;

ref_value          ::= 'REF' '(' ( variable | IDENT ) ')'
                     | 'NULL' ;

variable           ::= ( 'THIS' '.' )? path postfix* ;

literal            ::= INT | REAL | STRING | USTRING | CHAR_LIT
                     | DURATION | DATE | TOD | DT
                     | 'TRUE' | 'FALSE' ;
```

### 9.1 Precedence and associativity

From highest to lowest:

| Level | Operators | Associativity |
| --- | --- | --- |
| 11 | `(` `)` parentheses | — |
| 10 | call `f(...)` / method call | left |
| 9 | dereference `^`, subscripting `[]`, member access `.` | left |
| 8 | unary `+`, unary `-`, `NOT` | right |
| 7 | `**` | left |
| 6 | `*`, `/`, `MOD` | left |
| 5 | `+`, `-` | left |
| 4 | `=`, `<>`, `<`, `>`, `<=`, `>=` | left |
| 3 | `&`, `AND` | left |
| 2 | `XOR` | left |
| 1 | `OR` | left |

Notes:

- `STC-GRAM-009`: All comparison operators share precedence level 4 and are left-associative.
  This follows Table 71 of the 2013 standard. Annex A splits equality from relational comparison;
  `stc` resolves the inconsistency in favor of the operator table. A chained comparison such as
  `A < B < C` parses as `(A < B) < C` and is normally a type error because the left operand is
  `BOOL`.
- `STC-GRAM-010`: `**` is left-associative, matching the standard's left-to-right rule for equal
  precedence. `A ** B ** C` means `(A ** B) ** C`.
- `STC-GRAM-011`: Unary operators bind tighter than `**`, so `-2 ** 2` is `(-2) ** 2`. This
  follows the standard's precedence table.
- `STC-GRAM-012`: `&` and `AND` are the same operator; likewise `=` is equality, not assignment.
- `STC-GRAM-013`: Boolean `AND`/`&` and `OR` short-circuit left-to-right. The 2013 standard
  permits implementations to evaluate only as much as needed; `stc` chooses short-circuiting as
  a documented implementation-defined behavior. Function calls inside the skipped operand are
  not evaluated.
- `STC-GRAM-014`: Operands are otherwise evaluated left-to-right. There is no unspecified
  evaluation order exposed to the user.
- `STC-GRAM-015`: `NOT` is a unary operator at level 8. `NOT A = B` parses as `(NOT A) = B`;
  write `NOT (A = B)` for the other meaning.

### 9.2 Calls and parameters

- `f(a, b := 2)` — positional and named input parameters.
- `f(a => x)` — output parameter binding for FB/method calls.
- `f(NOT a => x)` — negated output binding, accepted from the 2013 grammar.
- `REF(v)` — reference value; `NULL` — null reference.
- A call may be qualified (`ns.f()`, `inst.m()`) and may use `THIS`/`SUPER` in a method body.
- The parser does not know whether a name denotes a function, FB, class method, or variable; it
  builds a call node whenever a path is followed by `(`. The analyzer resolves it and enforces:
  - functions may appear in expressions; FB calls may appear as statements;
  - a call that returns a value may be used as a statement (result discarded);
  - an FB call with a result used in an expression follows the 2013 rules (the result is the
    most recent assignment to the FB name).
- `SUPER()` with no arguments is a special FB-body call; `SUPER.method()` calls a base method.

### 9.3 Variable access and partial access

- `a.b` — struct/class member, FB output, or namespace member, resolved semantically.
- `a[1]`, `a[1, 2]` — array subscripts; multidimensional arrays use comma-separated indices.
- `a[1][2]` — an array of arrays, not a multidimensional array.
- `a^` — dereference a `REF_TO` value; multiple `^` are allowed only if the type is a reference
  to a reference type (a semantic check).
- `a.0` — partial bit access (2013 `Multibit_Part_Access`); `a.%X0`, `a.%B0`, etc. are the
  alternative forms. The bit range is checked against the underlying bit-string type.
- `THIS` is a reference to the current class/FB instance and may be passed to an interface
  variable. `THIS` cannot be applied to another instance.

## 10. UTF-8 extension grammar (`STC-UTF8`)

```ebnf
elementary_type_name ::= numeric_type_name
                       | bit_type_name
                       | string_type_name
                       | time_type_name
                       | date_type_name
                       | 'BOOL'
                       | 'USTRING' | 'UCHAR' ;   (* extension *)

string_type_name   ::= 'STRING' ( '[' INT ']' )?
                     | 'WSTRING' ( '[' INT ']' )?
                     | 'CHAR'
                     | 'WCHAR'
                     | 'USTRING' ( '[' INT ']' )?  (* extension *)
                     | 'UCHAR' ;                   (* extension *)

numeric_type_name  ::= int_type_name | 'REAL' | 'LREAL' ;
int_type_name      ::= signed_int_type_name | unsigned_int_type_name ;
signed_int_type_name ::= 'SINT' | 'INT' | 'DINT' | 'LINT' ;
unsigned_int_type_name ::= 'USINT' | 'UINT' | 'UDINT' | 'ULINT' ;
bit_type_name      ::= 'BOOL' | 'BYTE' | 'WORD' | 'DWORD' | 'LWORD' ;
time_type_name     ::= 'TIME' | 'LTIME' ;
date_type_name     ::= 'DATE' | 'LDATE' | 'TIME_OF_DAY' | 'TOD' | 'LTOD'
                     | 'LTIME_OF_DAY' | 'DATE_AND_TIME' | 'DT'
                     | 'LDT' | 'LDATE_AND_TIME' ;
```

The token classes `USTRING` and `CHAR_LIT` carry the decoded scalar sequence. The grammar does
not add new statement forms; `USTRING` values participate in the normal assignment, comparison,
array, struct, and call rules. See [Tokenization](Tokenization.md) §5.3 for literal decoding and
[Conformance](../01-Conformance-Profile.md) §5 for length/indexing semantics.

## 11. Disambiguation rules

These are resolved by the parser/analyzer, in addition to the EBNF:

1. **`PROGRAM`** is a POU declaration at top level and a program-configuration entry inside a
   `CONFIGURATION`. The parser is in a configuration context inside `CONFIGURATION`/`RESOURCE`.
2. **`TASK`** is a task declaration inside a resource and a name reference in `WITH`/interval
   positions. The keyword starts a declaration only at the beginning of a resource item.
3. **`ARRAY`** in `[` starts an array type/subscript; `*` inside `[` is a wildcard bound, while
   `*` elsewhere is multiplication.
4. **`:`** separates declaration names from types and `CASE` labels from statements. The parser
   tracks whether it is in a declaration, a case selection, or a configuration element.
5. **`AT`** is always the address-binding keyword in declarations; it is reserved and cannot be
   an identifier.
6. **`METHOD`** in an interface is a prototype (no body); in a class/FB it is a definition. The
   parser records the enclosing POU kind.
7. **A name followed by `(`** is a call, not a variable. `F` alone is a variable/type name; `F()`
   is a call. `REF(x)` is a reference value, not a normal call.
8. **`THIS`/`SUPER`** are only primary/call prefixes inside a method/FB body. `THIS` cannot be
   used as a standalone variable name.
9. **`END_*` matching**: the parser tracks the opening construct and reports mismatched
   terminators with both spans.
10. **Pragmas** attach to the following declaration/statement. A pragma at the end of a block
    attaches to the block terminator and is reported as unused if it has no meaning.

## 12. Semicolon and block rules (normative concession)

The 2013 grammar writes `Stmt_List : (Stmt? ';')*`, which requires a terminator after each
statement. Real-world ST commonly omits the semicolon after a block terminator. `stc` accepts
both forms:

- `IF ... END_IF;` and `IF ... END_IF` are both accepted when followed by another statement or a
  block delimiter.
- A simple statement (`x := 1`, a call, `RETURN`, `EXIT`, `CONTINUE`) requires `;`.
- `;;` is accepted as two empty statements.

This is a syntax superset, not a semantic change. The formatter always emits the semicolon after
block statements for consistency.

## 13. Constructs rejected with explicit diagnostics

| Construct | Diagnostic |
| --- | --- |
| IL body (`LD`, `ST`, `CAL`, ...) | `STC-E-GRAM-101` unsupported body language |
| LD/FBD/SFC body markers | `STC-E-GRAM-101` |
| `ACTION ... END_ACTION`, `STEP`, `TRANSITION` | `STC-E-GRAM-102` SFC not supported |
| Preprocessor directives `#if`, `#define`, `#include` | `STC-E-GRAM-103` no preprocessor |
| `PROPERTY` / Ed. 4 property declarations | `STC-E-GRAM-104` not in profile |
| `MUTEX`, `SEMA`, `ASSERT` (Ed. 4 additions) | `STC-E-GRAM-104` not in profile |
| Vendor pragmas with unknown meaning | warning `STC-W-PRAGMA-001` |
| `8#` octal | accepted + warning `STC-W-LEX-008` |

## 14. Grammar-level rule IDs (initial set)

| ID | Rule |
| --- | --- |
| `STC-GRAM-001` | Source unit composition |
| `STC-GRAM-002` | Namespaces and `USING` |
| `STC-GRAM-003` | Type declarations |
| `STC-GRAM-004` | Variable declarations |
| `STC-GRAM-005` | POU declarations and members |
| `STC-GRAM-006` | Configuration/resource/task declarations |
| `STC-GRAM-007` | Statements |
| `STC-GRAM-008` | Expressions |
| `STC-GRAM-009` | Comparison precedence |
| `STC-GRAM-010` | Power associativity |
| `STC-GRAM-011` | Unary vs. power precedence |
| `STC-GRAM-012` | Operator aliases |
| `STC-GRAM-013` | Boolean short-circuiting |
| `STC-GRAM-014` | Operand evaluation order |
| `STC-GRAM-015` | `NOT` precedence |
| `STC-GRAM-016` | Semicolon relaxation |

## 15. Open grammar questions

- Q-008: Unicode identifiers (currently rejected).
- Q-009: Comparison precedence interpretation (one level vs. the Annex A split).
- Q-010: Whether to keep the semicolon relaxation as a permanent syntax superset or require
  strict 2013 semicolons behind a `--strict` flag.
