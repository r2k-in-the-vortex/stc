/-
The surface AST for the core subset (P0.1).

The AST is an inductive family over declarations, expressions, statements, and
programs. Every node carries a source `Span`. `Ast.SpansNested` records the
structural invariant that a node's span contains its children's spans, and
`ast_span_wellformed` shows that a node and all of its descendants are
well-spanned and contained in the root. `Stc.Grammar.Derives` establishes the
invariant for grammatically derived trees.
-/
import Stc.Syntax.Tokens

namespace Stc.Syntax

/-- An elementary type name with its source span. Core type specs are exactly
the elementary type names; named, array, struct, enum, subrange, reference, and
pointer types are excluded by the Phase 0 scope. -/
structure TypeSpec where
  ty : ElementaryType
  span : Span

namespace TypeSpec

/-- Recover an elementary type spec from a type keyword token. -/
def ofToken? (t : Token) : Option TypeSpec :=
  match t.kind with
  | .keyword k =>
      match ElementaryType.ofKeyword? k with
      | some ty => some { ty := ty, span := t.span }
      | none => none
  | _ => none

/-- A recovered type spec carries the token's span. -/
theorem ofToken?_span {t : Token} {ts : TypeSpec} (h : ofToken? t = some ts) :
    ts.span = t.span := by
  cases t with
  | mk kind span =>
      cases kind with
      | keyword k =>
          cases k <;> simp [ofToken?, ElementaryType.ofKeyword?] at h
          all_goals (rw [← h])
      | _ => simp [ofToken?] at h

end TypeSpec

/-- The decoded payload of a literal. -/
inductive LiteralKind where
  /-- A decimal integer literal. -/
  | int (value : Int)
  /-- A based literal as a bit pattern. -/
  | based (value : Nat)
  /-- A typed integer or bit-string literal. -/
  | typedInt (ty : ElementaryType) (value : Int)
  /-- A boolean literal. -/
  | bool (value : Bool)
  /-- A typed character literal. -/
  | char (ty : CharType) (scalar : Nat)

/-- A literal with its source span. -/
structure Literal where
  kind : LiteralKind
  span : Span

namespace Literal

/-- Recover a literal from a lexical token. Keywords `TRUE` and `FALSE` are
boolean literals; typed boolean tokens carry the decoded value. -/
def ofToken? (t : Token) : Option Literal :=
  match t.kind with
  | .intLit value => some { kind := .int value, span := t.span }
  | .basedLit value => some { kind := .based value, span := t.span }
  | .typedInt ty value => some { kind := .typedInt ty value, span := t.span }
  | .typedBool value => some { kind := .bool value, span := t.span }
  | .charLit ty scalar => some { kind := .char ty scalar, span := t.span }
  | .keyword .kTrue => some { kind := .bool true, span := t.span }
  | .keyword .kFalse => some { kind := .bool false, span := t.span }
  | _ => none

/-- A recovered literal carries the token's span. -/
theorem ofToken?_span {t : Token} {l : Literal} (h : ofToken? t = some l) :
    l.span = t.span := by
  cases t with
  | mk kind span =>
      cases kind with
      | keyword k =>
          cases k <;> simp [ofToken?] at h
          all_goals (rw [← h])
      | _ =>
          simp [ofToken?] at h
          all_goals (rw [← h])

end Literal

/-- Unary operators in the core. -/
inductive UnaryOp where
  | neg
  | pos
  | not

/-- Binary operators in the core, including comparisons. `and` covers both the
keyword `AND` and the punctuation `&`; `eq` is equality, never assignment. -/
inductive BinaryOp where
  | or
  | xor
  | and
  | eq
  | ne
  | lt
  | gt
  | le
  | ge
  | add
  | sub
  | mul
  | div
  | mod
  | pow

/-- An assignable place. The core has scalar variable places only. -/
inductive Place where
  | var (name : Name) (span : Span)

namespace Place

/-- The source span of a place. -/
def span : Place → Span
  | .var _ s => s

end Place

mutual
  /-- A core expression. -/
  inductive Expr where
    /-- A literal. -/
    | literal (value : Literal) (span : Span)
    /-- A variable read. -/
    | var (name : Name) (span : Span)
    /-- A unary operation. -/
    | unary (op : UnaryOp) (operand : Expr) (span : Span)
    /-- A binary operation. -/
    | binary (op : BinaryOp) (lhs rhs : Expr) (span : Span)
    /-- A call. The callee is a simple name; qualified names are outside the
    core. Conversion functions such as `DINT(x)` are calls at this layer and
    are classified by typing in P0.2. -/
    | call (callee : Name) (args : List CallArg) (span : Span)
    /-- A parenthesized expression, preserved on the surface. -/
    | paren (operand : Expr) (span : Span)

  /-- A call argument. Named input arguments are included; output and in-out
  bindings are outside the core. -/
  inductive CallArg where
    | mk (name : Option Name) (value : Expr) (span : Span)
end

namespace Expr

/-- The source span of an expression. -/
def span : Expr → Span
  | .literal _ s => s
  | .var _ s => s
  | .unary _ _ s => s
  | .binary _ _ _ s => s
  | .call _ _ s => s
  | .paren _ s => s

/-- Replace the root span of an expression. The grammar relation uses this to
give each precedence-production wrapper the span of its CST production. -/
def withSpan (s : Span) : Expr → Expr
  | .literal l _ => .literal l s
  | .var n _ => .var n s
  | .unary op e _ => .unary op e s
  | .binary op l r _ => .binary op l r s
  | .call callee args _ => .call callee args s
  | .paren e _ => .paren e s

/-- Replacing the root span sets the expression's span to that span. -/
theorem withSpan_span (s : Span) (e : Expr) : (e.withSpan s).span = s := by
  cases e <;> rfl

end Expr

namespace CallArg

/-- The source span of a call argument. -/
def span : CallArg → Span
  | .mk _ _ s => s

/-- The optional parameter name. -/
def name? : CallArg → Option Name
  | .mk n _ _ => n

/-- The argument value. -/
def value : CallArg → Expr
  | .mk _ v _ => v

end CallArg

mutual
  /-- A core statement. Block bodies carry the span of their production. -/
  inductive Stmt where
    | assign (place : Place) (value : Expr) (span : Span)
    | call (call : Expr) (span : Span)
    | return_ (span : Span)
    | exit (span : Span)
    | continue_ (span : Span)
    | empty (span : Span)
    | ifThenElse (arms : List IfArm) (elseBody : Option StmtList) (span : Span)
    | case (selector : Expr) (arms : List CaseArm) (elseBody : Option StmtList) (span : Span)
    | forLoop (var : Name) (start stop : Expr) (step : Option Expr) (body : StmtList) (span : Span)
    | whileLoop (cond : Expr) (body : StmtList) (span : Span)
    | repeatLoop (body : StmtList) (cond : Expr) (span : Span)

  /-- A statement list. The grammar requires a semicolon after every statement;
  an empty statement is represented by `Stmt.empty`. -/
  inductive StmtList where
    | mk (stmts : List Stmt) (span : Span)

  /-- An `IF` or `ELSIF` arm. -/
  inductive IfArm where
    | mk (cond : Expr) (body : StmtList) (span : Span)

  /-- A `CASE` label, optionally a range. -/
  inductive CaseLabel where
    | mk (low : Expr) (high : Option Expr) (span : Span)

  /-- A `CASE` selection: one or more labels and a body. -/
  inductive CaseArm where
    | mk (labels : List CaseLabel) (body : StmtList) (span : Span)
end

namespace Stmt

/-- The source span of a statement. -/
def span : Stmt → Span
  | .assign _ _ s => s
  | .call _ s => s
  | .return_ s => s
  | .exit s => s
  | .continue_ s => s
  | .empty s => s
  | .ifThenElse _ _ s => s
  | .case _ _ _ s => s
  | .forLoop _ _ _ _ _ s => s
  | .whileLoop _ _ s => s
  | .repeatLoop _ _ s => s

end Stmt

namespace StmtList

/-- The source span of a statement list. -/
def span : StmtList → Span
  | .mk _ s => s

/-- The statements in a list. -/
def stmts : StmtList → List Stmt
  | .mk ss _ => ss

end StmtList

namespace IfArm

/-- The source span of an `IF`/`ELSIF` arm. -/
def span : IfArm → Span
  | .mk _ _ s => s

/-- The arm condition. -/
def cond : IfArm → Expr
  | .mk c _ _ => c

/-- The arm body. -/
def body : IfArm → StmtList
  | .mk _ b _ => b

end IfArm

namespace CaseLabel

/-- The source span of a `CASE` label. -/
def span : CaseLabel → Span
  | .mk _ _ s => s

/-- The low (or only) label bound. -/
def low : CaseLabel → Expr
  | .mk l _ _ => l

/-- The optional high bound of a range label. -/
def high : CaseLabel → Option Expr
  | .mk _ h _ => h

end CaseLabel

namespace CaseArm

/-- The source span of a `CASE` selection. -/
def span : CaseArm → Span
  | .mk _ _ s => s

/-- The labels of a selection. -/
def labels : CaseArm → List CaseLabel
  | .mk ls _ _ => ls

/-- The selection body. -/
def body : CaseArm → StmtList
  | .mk _ b _ => b

end CaseArm

/-- A variable declaration: one or more names, a type, and an optional scalar
initializer. -/
structure VarDecl where
  names : List Name
  ty : TypeSpec
  init : Option Expr
  span : Span

/-- The variable-section kinds in the core: ordinary locals, function inputs,
and global roots. `VAR_OUTPUT`, `VAR_IN_OUT`, `VAR_TEMP`, `VAR_EXTERNAL`,
qualifiers, and `AT` bindings are outside the core. -/
inductive VarKind where
  | var
  | input
  | global

/-- A variable section. -/
structure VarSection where
  kind : VarKind
  decls : List VarDecl
  span : Span

/-- A core function declaration. -/
structure FunctionDecl where
  name : Name
  resultType : Option TypeSpec
  sections : List VarSection
  body : StmtList
  span : Span

/-- A core program declaration. -/
structure ProgramDecl where
  name : Name
  sections : List VarSection
  body : StmtList
  span : Span

/-- A top-level declaration in the core. -/
inductive TopDecl where
  | global (globals : VarSection)
  | function (decl : FunctionDecl)
  | program (decl : ProgramDecl)

/-- A source unit: a sequence of global sections, functions, and programs. The
phase semantics restricts this to one program; the surface model does not. -/
structure SourceUnit where
  decls : List TopDecl
  span : Span

/-- A non-empty argument list with its span. -/
structure ArgumentList where
  args : List CallArg
  span : Span

/-- A non-empty case-label list with its span. -/
structure CaseLabelList where
  labels : List CaseLabel
  span : Span

/-- A tagged view over every AST node. It gives the grammar relation,
span relation, and later phases one uniform carrier for AST nodes. -/
inductive Ast where
  | unit (value : SourceUnit)
  | top (value : TopDecl)
  | varSection (value : VarSection)
  | varDecl (value : VarDecl)
  | function (value : FunctionDecl)
  | program (value : ProgramDecl)
  | typeSpec (value : TypeSpec)
  | name (value : Name)
  | stmt (value : Stmt)
  | stmtList (value : StmtList)
  | expr (value : Expr)
  | literal (value : Literal)
  | place (value : Place)
  | callArg (value : CallArg)
  | argList (value : ArgumentList)
  | ifArm (value : IfArm)
  | caseArm (value : CaseArm)
  | caseLabel (value : CaseLabel)
  | caseLabelList (value : CaseLabelList)

namespace Ast

/-- The source span of any AST node. -/
def span : Ast → Span
  | .unit u => u.span
  | .top (.global s) => s.span
  | .top (.function f) => f.span
  | .top (.program p) => p.span
  | .varSection s => s.span
  | .varDecl d => d.span
  | .function f => f.span
  | .program p => p.span
  | .typeSpec t => t.span
  | .name n => n.span
  | .stmt s => s.span
  | .stmtList l => l.span
  | .expr e => e.span
  | .literal l => l.span
  | .place p => p.span
  | .callArg a => a.span
  | .argList l => l.span
  | .ifArm a => a.span
  | .caseArm a => a.span
  | .caseLabel l => l.span
  | .caseLabelList l => l.span

/-- The immediate AST children of a node. Leaves have no children. -/
def children : Ast → List Ast
  | .unit u => u.decls.map .top
  | .top (.global s) => [.varSection s]
  | .top (.function f) => [.function f]
  | .top (.program p) => [.program p]
  | .varSection s => s.decls.map .varDecl
  | .varDecl d =>
      d.names.map .name ++ [.typeSpec d.ty] ++ d.init.toList.map .expr
  | .function f =>
      [.name f.name] ++ f.resultType.toList.map .typeSpec ++
        f.sections.map .varSection ++ [.stmtList f.body]
  | .program p =>
      [.name p.name] ++ p.sections.map .varSection ++ [.stmtList p.body]
  | .typeSpec _ => []
  | .name _ => []
  | .stmt (.assign p e _) => [.place p, .expr e]
  | .stmt (.call e _) => [.expr e]
  | .stmt (.return_ _) => []
  | .stmt (.exit _) => []
  | .stmt (.continue_ _) => []
  | .stmt (.empty _) => []
  | .stmt (.ifThenElse arms elseBody _) =>
      arms.map .ifArm ++ elseBody.toList.map .stmtList
  | .stmt (.case selector arms elseBody _) =>
      [.expr selector] ++ arms.map .caseArm ++ elseBody.toList.map .stmtList
  | .stmt (.forLoop var start stop step body _) =>
      [.name var, .expr start, .expr stop] ++ step.toList.map .expr ++ [.stmtList body]
  | .stmt (.whileLoop cond body _) => [.expr cond, .stmtList body]
  | .stmt (.repeatLoop body cond _) => [.stmtList body, .expr cond]
  | .stmtList l => l.stmts.map .stmt
  | .expr (.literal l _) => [.literal l]
  | .expr (.var n _) => [.name n]
  | .expr (.unary _ e _) => [.expr e]
  | .expr (.binary _ l r _) => [.expr l, .expr r]
  | .expr (.call callee args _) => [.name callee] ++ args.map .callArg
  | .expr (.paren e _) => [.expr e]
  | .literal _ => []
  | .place (.var n _) => [.name n]
  | .callArg a =>
      (match a.name? with | some n => [.name n] | none => []) ++ [.expr a.value]
  | .argList l => l.args.map .callArg
  | .ifArm a => [.expr a.cond, .stmtList a.body]
  | .caseArm a => a.labels.map .caseLabel ++ [.stmtList a.body]
  | .caseLabel l => [.expr l.low] ++ l.high.toList.map .expr
  | .caseLabelList l => l.labels.map .caseLabel

/-- The descendant relation: immediate children and its transitive closure. -/
inductive Descendant : Ast → Ast → Prop where
  | child {a c : Ast} (h : c ∈ a.children) : Descendant a c
  | trans {a b c : Ast} (h1 : Descendant a b) (h2 : Descendant b c) : Descendant a c

/-- Every node's span is well formed and contains its children's spans. -/
inductive SpansNested : Ast → Prop where
  | mk {a : Ast} :
      a.span.WellFormed →
      (∀ c, c ∈ a.children → a.span.contains c.span) →
      (∀ c, c ∈ a.children → c.SpansNested) →
      SpansNested a

/-- A node whose spans are nested is itself well-spanned. -/
theorem SpansNested.span_wellFormed {a : Ast} (h : a.SpansNested) :
    a.span.WellFormed := by
  cases h with
  | mk hvalid _ _ => exact hvalid

end Ast

/-- `ast_span_wellformed`: every descendant of a span-nested AST node is
well-spanned and contained in the root node's span. Combined with
`Stc.Grammar.Derives`, this is the source-span mapping used by Phase 4. -/
theorem ast_span_wellformed {a b : Ast} (hd : a.Descendant b) :
    a.SpansNested → b.SpansNested ∧ a.span.contains b.span := by
  induction hd with
  | child hmem =>
      intro ha
      cases ha with
      | mk _ hcontain hnested =>
          exact ⟨hnested _ hmem, hcontain _ hmem⟩
  | trans _ _ ih1 ih2 =>
      intro ha
      have h1 := ih1 ha
      have h2 := ih2 h1.1
      exact ⟨h2.1, Span.contains_trans h1.2 h2.2⟩

end Stc.Syntax
