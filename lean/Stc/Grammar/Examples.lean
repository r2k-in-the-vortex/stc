/-
A small, checked derivation for the core grammar relation (P0.1).

The example derives

    PROGRAM P
    VAR
      x : DINT;
    END_VAR
      x := 1;
    END_PROGRAM

from a hand-built CST. It is the P0.1 smoke test: every declaration,
statement, and expression production used by the example is exercised by the
`Derives` constructors. It is not a parser test and makes no claim that the
Rust parser produces this CST. All nodes use the same span; the example checks
rule shapes, not the lossless coverage predicate.
-/
import Stc.Grammar.Derives

namespace Stc.Grammar.Examples

open Stc.Syntax

/-- A token of a given kind at a span (stand-in for a lexer result). -/
def tok (span : Span) (kind : TokenKind) : Token :=
  { kind := kind, span := span }

/-- A simple path node. -/
def pathNode (span : Span) (name : String) : Cst :=
  .node .path span [.token (tok span (.ident name))]

/-- A `DINT` type node. -/
def dintNode (span : Span) : Cst :=
  .node .typeSpec span [.token (tok span (.keyword .kDint))]

/-- The literal `1`. -/
def litNode (span : Span) : Cst :=
  .node .literalExpression span [.token (tok span (.intLit 1))]

def primaryNode (span : Span) : Cst :=
  .node .primaryExpression span [litNode span]

def postfixNode (span : Span) : Cst :=
  .node .postfixExpression span [primaryNode span]

def unaryNode (span : Span) : Cst :=
  .node .unaryExpression span [postfixNode span]

def powerNode (span : Span) : Cst :=
  .node .powerExpression span [unaryNode span]

def termNode (span : Span) : Cst :=
  .node .term span [powerNode span]

def addNode (span : Span) : Cst :=
  .node .addExpression span [termNode span]

def equNode (span : Span) : Cst :=
  .node .equExpression span [addNode span]

def compareNode (span : Span) : Cst :=
  .node .compareExpression span [equNode span]

def andNode (span : Span) : Cst :=
  .node .andExpression span [compareNode span]

def xorNode (span : Span) : Cst :=
  .node .xorExpression span [andNode span]

def orNode (span : Span) : Cst :=
  .node .orExpression span [xorNode span]

/-- The expression `1`, wrapped through every precedence production. -/
def exprNode (span : Span) : Cst :=
  .node .expression span [orNode span]

/-- The statement `x := 1;`. -/
def assignNode (span : Span) : Cst :=
  let variableC : Cst := .node .variable span [pathNode span "x"]
  let placeC : Cst := .node .place span [variableC]
  let stmtC : Cst := .node .assignmentStatement span
    [placeC, .token (tok span (.punct .assign)), exprNode span]
  .node .statementList span
    [stmtC, .token (tok span (.punct .semicolon))]

/-- The variable declaration `x : DINT;`. -/
def varDeclNode (span : Span) : Cst :=
  .node .varDecl span
    [pathNode span "x",
      .token (tok span (.punct .colon)),
      dintNode span,
      .token (tok span (.punct .semicolon))]

/-- The `VAR ... END_VAR` section. -/
def varSectionNode (span : Span) : Cst :=
  .node .varSection span
    [.token (tok span (.keyword .kVar)),
      varDeclNode span,
      .token (tok span (.keyword .kEndVar))]

/-- The `PROGRAM P ... END_PROGRAM` declaration. -/
def programNode (span : Span) : Cst :=
  .node .programDecl span
    [.token (tok span (.keyword .kProgram)),
      pathNode span "P",
      varSectionNode span,
      assignNode span,
      .token (tok span (.keyword .kEndProgram))]

/-- The top item and source file around the program. -/
def sourceNode (span : Span) : Cst :=
  let topC : Cst := .node .topItem span [programNode span]
  .node .sourceFile span [topC, .token (tok span .eof)]

/-- The AST side of the example. -/
def name (span : Span) (text : String) : Name :=
  { text := text, span := span }

/-- The expression AST for `1`. -/
def intOneAst (span : Span) : Expr :=
  .literal { kind := .int 1, span := span } span

/-- The variable declaration AST. -/
def varDeclAst (span : Span) : VarDecl :=
  { names := [name span "x"],
    ty := { ty := .dint, span := span },
    init := none,
    span := span }

/-- The program AST. -/
def programAst (span : Span) : ProgramDecl :=
  let body : StmtList := .mk
    [ .assign (.var (name span "x") span) (intOneAst span) span ]
    span
  { name := name span "P",
    sections := [{ kind := .var, decls := [varDeclAst span], span := span }],
    body := body,
    span := span }

/-- The source-unit AST. -/
def unitAst (span : Span) : SourceUnit :=
  { decls := [.program (programAst span)], span := span }

example (span : Span) : Derives (sourceNode span) (.unit (unitAst span)) := by
  have hlit : Derives (litNode span)
      (.literal { kind := .int 1, span := span }) := by
    unfold litNode tok
    exact Derives.literalExpression rfl
  have hprimary : Derives (primaryNode span) (.expr (intOneAst span)) := by
    unfold primaryNode intOneAst
    exact Derives.primaryLiteral hlit
  have hpostfix : Derives (postfixNode span) (.expr (intOneAst span)) := by
    unfold postfixNode
    exact Derives.postfixExpression hprimary
  have hunary : Derives (unaryNode span) (.expr (intOneAst span)) := by
    unfold unaryNode
    exact Derives.unaryOperand hpostfix
  have hpower : Derives (powerNode span) (.expr (intOneAst span)) := by
    unfold powerNode
    exact Derives.powerExpression hunary (DerivesChained.done _)
  have hterm : Derives (termNode span) (.expr (intOneAst span)) := by
    unfold termNode
    exact Derives.term hpower (DerivesChained.done _)
  have hadd : Derives (addNode span) (.expr (intOneAst span)) := by
    unfold addNode
    exact Derives.addExpression hterm (DerivesChained.done _)
  have hequ : Derives (equNode span) (.expr (intOneAst span)) := by
    unfold equNode
    exact Derives.equExpression hadd (DerivesChained.done _)
  have hcompare : Derives (compareNode span) (.expr (intOneAst span)) := by
    unfold compareNode
    exact Derives.compareExpression hequ (DerivesChained.done _)
  have hand : Derives (andNode span) (.expr (intOneAst span)) := by
    unfold andNode
    exact Derives.andExpression hcompare (DerivesChained.done _)
  have hxor : Derives (xorNode span) (.expr (intOneAst span)) := by
    unfold xorNode
    exact Derives.xorExpression hand (DerivesChained.done _)
  have hor : Derives (orNode span) (.expr (intOneAst span)) := by
    unfold orNode
    exact Derives.orExpression hxor (DerivesChained.done _)
  have hexpr : Derives (exprNode span) (.expr (intOneAst span)) := by
    unfold exprNode
    exact Derives.expression hor
  have hvariable : Derives (.node .variable span [pathNode span "x"])
      (.place (.var (name span "x") span)) := by
    unfold pathNode tok name
    exact Derives.variable (Derives.path rfl)
  have hplace : Derives (.node .place span [.node .variable span [pathNode span "x"]])
      (.place (.var (name span "x") span)) := by
    unfold pathNode tok name
    exact Derives.place (Derives.variable (Derives.path rfl))
  have hassign : Derives
      (.node .assignmentStatement span
        [.node .place span [.node .variable span [pathNode span "x"]],
          .token (tok span (.punct .assign)), exprNode span])
      (.stmt (.assign (.var (name span "x") span) (intOneAst span) span)) :=
    Derives.assignmentStatement hplace rfl hexpr
  have hstmtList : StatementListRel
      (fun c s => Derives c (.stmt s))
      [.node .assignmentStatement span
        [.node .place span [.node .variable span [pathNode span "x"]],
          .token (tok span (.punct .assign)), exprNode span],
        .token (tok span (.punct .semicolon))]
      [.assign (.var (name span "x") span) (intOneAst span) span] :=
    StatementListRel.stmt hassign rfl StatementListRel.nil
  have hbody : Derives (assignNode span)
      (.stmtList (.mk [.assign (.var (name span "x") span) (intOneAst span) span] span)) := by
    unfold assignNode exprNode orNode xorNode andNode compareNode equNode addNode termNode
      powerNode unaryNode postfixNode primaryNode litNode pathNode tok name intOneAst
    exact Derives.statementList hstmtList
  have hvarDecl : Derives (varDeclNode span) (.varDecl (varDeclAst span)) := by
    unfold varDeclNode varDeclAst pathNode dintNode tok name
    exact Derives.varDecl
      (CommaSeparated.single (Derives.path rfl)) rfl
      (Derives.typeSpec rfl) PunctNodeOption.none rfl
  have hvarSection : Derives (varSectionNode span)
      (.varSection ⟨.var, [varDeclAst span], span⟩) := by
    unfold varSectionNode varDeclNode varDeclAst pathNode dintNode tok name
    exact Derives.varSection rfl rfl rfl (ListRel.cons hvarDecl ListRel.nil)
  have hprogram : Derives (programNode span) (.program (programAst span)) := by
    unfold programNode programAst varSectionNode varDeclNode varDeclAst pathNode
      dintNode tok name intOneAst
    exact Derives.programDecl rfl (Derives.path rfl)
      (ListRel.cons hvarSection ListRel.nil) hbody rfl
  have htop : Derives (.node .topItem span [programNode span])
      (.top (.program (programAst span))) := by
    exact Derives.topItemProgram hprogram
  unfold sourceNode unitAst
  exact Derives.sourceFile rfl (ListRel.cons htop ListRel.nil)

end Stc.Grammar.Examples
