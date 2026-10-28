/-
`Derives : Cst -> Ast -> Prop`, the declarative core grammar relation
(P0.1).

Every core production in `Plan-Phase0.md` §3.3 receives a constructor or a
relation rule, and the coverage table recorded at P0.1 maps each rule to its
specification production. Phase 0 makes no parser-correctness claim; Phase 4
proves the parser contracts against this relation.

The relation is defined over the trivia-free projection of a CST. A full CST
maps to an AST when its `Cst.stripTrivia` projection derives it; the lossless
coverage contract stays on the full tree.
-/
import Stc.Syntax.Ast
import Stc.Syntax.Cst

namespace Stc.Grammar

open Stc.Syntax

/-- The list relation used for repeated syntax elements. -/
inductive ListRel {α β : Type} (R : α → β → Prop) : List α → List β → Prop where
  | nil : ListRel R [] []
  | cons : R a b → ListRel R as bs → ListRel R (a :: as) (b :: bs)

/-- The non-empty list relation used for `+` repetitions. -/
inductive ListRel1 {α β : Type} (R : α → β → Prop) : List α → List β → Prop where
  | single : R a b → ListRel1 R [a] [b]
  | cons : R a b → ListRel1 R as bs → ListRel1 R (a :: as) (b :: bs)

/-- An optional single node. -/
inductive OptionalNode {α : Type} (R : Cst → α → Prop) : List Cst → Option α → Prop where
  | none : OptionalNode R [] none
  | some : R c a → OptionalNode R [c] (some a)

/-- An optional `punctuation node` pair, such as `':' type_spec` or
`':=' expression`. -/
inductive PunctNodeOption {α : Type} (p : Punct) (R : Cst → α → Prop) :
    List Cst → Option α → Prop where
  | none : PunctNodeOption p R [] none
  | some {t : Token} {c : Cst} {a : α} :
      t.kind = .punct p → R c a → PunctNodeOption p R [.token t, c] (some a)

/-- An optional `keyword node` pair, such as `BY expression`. -/
inductive KeywordNodeOption {α : Type} (k : Keyword) (R : Cst → α → Prop) :
    List Cst → Option α → Prop where
  | none : KeywordNodeOption k R [] none
  | some {t : Token} {c : Cst} {a : α} :
      t.kind = .keyword k → R c a → KeywordNodeOption k R [.token t, c] (some a)

/-- A non-empty comma-separated list of nodes. -/
inductive CommaSeparated {α : Type} (R : Cst → α → Prop) : List Cst → List α → Prop where
  | single {c : Cst} {a : α} : R c a → CommaSeparated R [c] [a]
  | cons {c : Cst} {a : α} {rest : List Cst} {as : List α} {comma : Token} :
      R c a →
      comma.kind = .punct .comma →
      CommaSeparated R rest as →
      CommaSeparated R (c :: .token comma :: rest) (a :: as)

/-- A statement list: `( stmt? ';' )*`. An omitted statement is an empty
statement, represented by the semicolon's span. -/
inductive StatementListRel (R : Cst → Stmt → Prop) : List Cst → List Stmt → Prop where
  | nil : StatementListRel R [] []
  | stmt {c : Cst} {s : Stmt} {semi : Token} {rest : List Cst} {ss : List Stmt} :
      R c s →
      semi.kind = .punct .semicolon →
      StatementListRel R rest ss →
      StatementListRel R (c :: .token semi :: rest) (s :: ss)
  | empty {semi : Token} {rest : List Cst} {ss : List Stmt} :
      semi.kind = .punct .semicolon →
      StatementListRel R rest ss →
      StatementListRel R (.token semi :: rest) (Stmt.empty semi.span :: ss)

/-- A left-associated chain of binary operators. The final expression is the
accumulated left subtree; each step unions the operand spans. -/
inductive DerivesChained (operand : Cst → Expr → Prop) (opOf : Token → Option BinaryOp) :
    List Cst → Expr → Expr → Prop where
  | done (acc : Expr) : DerivesChained operand opOf [] acc acc
  | step {tok : Token} {c : Cst} {rest : List Cst} {acc e acc' : Expr} {op : BinaryOp} :
      opOf tok = some op →
      operand c e →
      DerivesChained operand opOf rest
        (Expr.binary op acc e (acc.span.union e.span)) acc' →
      DerivesChained operand opOf (.token tok :: c :: rest) acc acc'

/-- The variable-section kind named by a keyword. -/
def Keyword.varKind? : Keyword → Option VarKind
  | .kVar => some .var
  | .kVarInput => some .input
  | .kVarGlobal => some .global
  | _ => none

/-- A unary operator token. -/
def Token.unaryOp? (t : Token) : Option UnaryOp :=
  match t.kind with
  | .punct .minus => some .neg
  | .punct .plus => some .pos
  | .keyword .kNot => some .not
  | _ => none

/-- `OR` at the `or_expr` level. -/
def Token.orOp? (t : Token) : Option BinaryOp :=
  match t.kind with
  | .keyword .kOr => some .or
  | _ => none

/-- `XOR` at the `xor_expr` level. -/
def Token.xorOp? (t : Token) : Option BinaryOp :=
  match t.kind with
  | .keyword .kXor => some .xor
  | _ => none

/-- `AND` and `&` at the `and_expr` level. -/
def Token.andOp? (t : Token) : Option BinaryOp :=
  match t.kind with
  | .keyword .kAnd => some .and
  | .punct .ampersand => some .and
  | _ => none

/-- Equality operators at the `compare_expr` level. -/
def Token.compareOp? (t : Token) : Option BinaryOp :=
  match t.kind with
  | .punct .equal => some .eq
  | .punct .notEqual => some .ne
  | _ => none

/-- Relational operators at the `equ_expr` level. -/
def Token.equOp? (t : Token) : Option BinaryOp :=
  match t.kind with
  | .punct .less => some .lt
  | .punct .greater => some .gt
  | .punct .lessEqual => some .le
  | .punct .greaterEqual => some .ge
  | _ => none

/-- Additive operators at the `add_expr` level. -/
def Token.addOp? (t : Token) : Option BinaryOp :=
  match t.kind with
  | .punct .plus => some .add
  | .punct .minus => some .sub
  | _ => none

/-- Multiplicative operators at the `term` level. -/
def Token.termOp? (t : Token) : Option BinaryOp :=
  match t.kind with
  | .punct .star => some .mul
  | .punct .slash => some .div
  | .keyword .kMod => some .mod
  | _ => none

/-- The power operator at the `power_expr` level. -/
def Token.powerOp? (t : Token) : Option BinaryOp :=
  match t.kind with
  | .punct .power => some .pow
  | _ => none

/-- A derivation of an AST node from a trivia-free CST. -/
inductive Derives : Cst → Ast → Prop where
  -- Names and types ------------------------------------------------------
  | path {span : Span} {tok : Token} {name : Name} :
      Name.ofToken? tok = some name →
      Derives (.node .path span [.token tok]) (.name name)
  | typeSpec {span : Span} {tok : Token} {ty : TypeSpec} :
      TypeSpec.ofToken? tok = some ty →
      Derives (.node .typeSpec span [.token tok]) (.typeSpec ty)

  -- Source unit and top-level declarations -------------------------------
  | sourceFile {span : Span} {topNodes : List Cst} {eof : Token}
      {decls : List TopDecl} :
      eof.kind = .eof →
      ListRel (fun c t => Derives c (.top t)) topNodes decls →
      Derives (.node .sourceFile span (topNodes ++ [.token eof]))
        (.unit ⟨decls, span⟩)
  | topItemGlobal {span : Span} {child : Cst} {globals : VarSection} :
      Derives child (.varSection globals) →
      globals.kind = .global →
      Derives (.node .topItem span [child]) (.top (.global globals))
  | topItemFunction {span : Span} {child : Cst} {decl : FunctionDecl} :
      Derives child (.function decl) →
      Derives (.node .topItem span [child]) (.top (.function decl))
  | topItemProgram {span : Span} {child : Cst} {decl : ProgramDecl} :
      Derives child (.program decl) →
      Derives (.node .topItem span [child]) (.top (.program decl))
  | varSection {span : Span} {kindTok endTok : Token} {declNodes : List Cst}
      {decls : List VarDecl} {kw : Keyword} {kind : VarKind} :
      kindTok.kind = .keyword kw →
      Keyword.varKind? kw = some kind →
      endTok.kind = .keyword .kEndVar →
      ListRel (fun c d => Derives c (.varDecl d)) declNodes decls →
      Derives (.node .varSection span
          (.token kindTok :: declNodes ++ [.token endTok]))
        (.varSection ⟨kind, decls, span⟩)
  | varDecl {span : Span} {nameNodes : List Cst} {names : List Name}
      {colon : Token} {typeNode : Cst} {ty : TypeSpec}
      {initNodes : List Cst} {init : Option Expr} {semi : Token} :
      CommaSeparated (fun c n => Derives c (.name n)) nameNodes names →
      colon.kind = .punct .colon →
      Derives typeNode (.typeSpec ty) →
      PunctNodeOption .assign (fun c e => Derives c (.expr e)) initNodes init →
      semi.kind = .punct .semicolon →
      Derives (.node .varDecl span
          (nameNodes ++ [.token colon, typeNode] ++ initNodes ++ [.token semi]))
        (.varDecl ⟨names, ty, init, span⟩)
  | functionDecl {span : Span} {fnTok endTok : Token} {nameNode : Cst}
      {name : Name} {resultNodes : List Cst} {result : Option TypeSpec}
      {sectionNodes : List Cst} {sections : List VarSection}
      {bodyNode : Cst} {body : StmtList} :
      fnTok.kind = .keyword .kFunction →
      Derives nameNode (.name name) →
      PunctNodeOption .colon (fun c t => Derives c (.typeSpec t)) resultNodes result →
      ListRel (fun c s => Derives c (.varSection s)) sectionNodes sections →
      Derives bodyNode (.stmtList body) →
      endTok.kind = .keyword .kEndFunction →
      Derives (.node .functionDecl span
          (.token fnTok :: nameNode :: resultNodes ++ sectionNodes ++
            [bodyNode, .token endTok]))
        (.function ⟨name, result, sections, body, span⟩)
  | programDecl {span : Span} {programTok endTok : Token} {nameNode : Cst}
      {name : Name} {sectionNodes : List Cst} {sections : List VarSection}
      {bodyNode : Cst} {body : StmtList} :
      programTok.kind = .keyword .kProgram →
      Derives nameNode (.name name) →
      ListRel (fun c s => Derives c (.varSection s)) sectionNodes sections →
      Derives bodyNode (.stmtList body) →
      endTok.kind = .keyword .kEndProgram →
      Derives (.node .programDecl span
          (.token programTok :: nameNode :: sectionNodes ++
            [bodyNode, .token endTok]))
        (.program ⟨name, sections, body, span⟩)

  -- Statements -----------------------------------------------------------
  | statementList {span : Span} {children : List Cst} {stmts : List Stmt} :
      StatementListRel (fun c s => Derives c (.stmt s)) children stmts →
      Derives (.node .statementList span children) (.stmtList (.mk stmts span))
  | assignmentStatement {span : Span} {placeNode : Cst} {place : Place}
      {assignTok : Token} {valueNode : Cst} {value : Expr} :
      Derives placeNode (.place place) →
      assignTok.kind = .punct .assign →
      Derives valueNode (.expr value) →
      Derives (.node .assignmentStatement span [placeNode, .token assignTok, valueNode])
        (.stmt (.assign place value span))
  | callStatement {span : Span} {callNode : Cst} {call : Expr} :
      Derives callNode (.expr call) →
      Derives (.node .callStatement span [callNode]) (.stmt (.call call span))
  | returnStatement {span : Span} {tok : Token} :
      tok.kind = .keyword .kReturn →
      Derives (.node .returnStatement span [.token tok]) (.stmt (.return_ span))
  | exitStatement {span : Span} {tok : Token} :
      tok.kind = .keyword .kExit →
      Derives (.node .exitStatement span [.token tok]) (.stmt (.exit span))
  | continueStatement {span : Span} {tok : Token} :
      tok.kind = .keyword .kContinue →
      Derives (.node .continueStatement span [.token tok]) (.stmt (.continue_ span))
  | ifStatement {span : Span} {ifTok thenTok endIfTok : Token}
      {condNode : Cst} {cond : Expr} {bodyNode : Cst} {body : StmtList}
      {elsifNodes : List Cst} {elsifArms : List IfArm}
      {elseNodes : List Cst} {elseBody : Option StmtList} :
      ifTok.kind = .keyword .kIf →
      Derives condNode (.expr cond) →
      thenTok.kind = .keyword .kThen →
      Derives bodyNode (.stmtList body) →
      ListRel (fun c a => Derives c (.ifArm a)) elsifNodes elsifArms →
      OptionalNode (fun c l => Derives c (.stmtList l)) elseNodes elseBody →
      endIfTok.kind = .keyword .kEndIf →
      Derives (.node .ifStatement span
          ([.token ifTok, condNode, .token thenTok, bodyNode] ++
            elsifNodes ++ elseNodes ++ [.token endIfTok]))
        (.stmt (.ifThenElse (IfArm.mk cond body span :: elsifArms) elseBody span))
  | elsifClause {span : Span} {elsifTok thenTok : Token}
      {condNode : Cst} {cond : Expr} {bodyNode : Cst} {body : StmtList} :
      elsifTok.kind = .keyword .kElsif →
      Derives condNode (.expr cond) →
      thenTok.kind = .keyword .kThen →
      Derives bodyNode (.stmtList body) →
      Derives (.node .elsifClause span
          [.token elsifTok, condNode, .token thenTok, bodyNode])
        (.ifArm (.mk cond body span))
  | elseClause {span : Span} {elseTok : Token} {bodyNode : Cst} {body : StmtList} :
      elseTok.kind = .keyword .kElse →
      Derives bodyNode (.stmtList body) →
      Derives (.node .elseClause span [.token elseTok, bodyNode]) (.stmtList body)
  | caseStatement {span : Span} {caseTok ofTok endCaseTok : Token}
      {selectorNode : Cst} {selector : Expr}
      {selectionNodes : List Cst} {arms : List CaseArm}
      {elseNodes : List Cst} {elseBody : Option StmtList} :
      caseTok.kind = .keyword .kCase →
      Derives selectorNode (.expr selector) →
      ofTok.kind = .keyword .kOf →
      ListRel1 (fun c a => Derives c (.caseArm a)) selectionNodes arms →
      OptionalNode (fun c l => Derives c (.stmtList l)) elseNodes elseBody →
      endCaseTok.kind = .keyword .kEndCase →
      Derives (.node .caseStatement span
          ([.token caseTok, selectorNode, .token ofTok] ++
            selectionNodes ++ elseNodes ++ [.token endCaseTok]))
        (.stmt (.case selector arms elseBody span))
  | caseSelection {span : Span} {listNode : Cst} {labels : CaseLabelList}
      {colon : Token} {bodyNode : Cst} {body : StmtList} :
      Derives listNode (.caseLabelList labels) →
      colon.kind = .punct .colon →
      Derives bodyNode (.stmtList body) →
      Derives (.node .caseSelection span [listNode, .token colon, bodyNode])
        (.caseArm (.mk labels.labels body span))
  | caseList {span : Span} {children : List Cst} {labels : List CaseLabel} :
      CommaSeparated (fun c l => Derives c (.caseLabel l)) children labels →
      Derives (.node .caseList span children) (.caseLabelList ⟨labels, span⟩)
  | caseElementSingle {span : Span} {eNode : Cst} {e : Expr} :
      Derives eNode (.expr e) →
      Derives (.node .caseElement span [eNode]) (.caseLabel (.mk e none span))
  | caseElementRange {span : Span} {lowNode : Cst} {low : Expr}
      {rangeTok : Token} {highNode : Cst} {high : Expr} :
      Derives lowNode (.expr low) →
      rangeTok.kind = .punct .range →
      Derives highNode (.expr high) →
      Derives (.node .caseElement span [lowNode, .token rangeTok, highNode])
        (.caseLabel (.mk low (some high) span))
  | forStatement {span : Span} {forTok assignTok toTok doTok endForTok : Token}
      {varNode : Cst} {var : Name} {startNode : Cst} {start : Expr}
      {stopNode : Cst} {stop : Expr} {stepNodes : List Cst} {step : Option Expr}
      {bodyNode : Cst} {body : StmtList} :
      forTok.kind = .keyword .kFor →
      Derives varNode (.name var) →
      assignTok.kind = .punct .assign →
      Derives startNode (.expr start) →
      toTok.kind = .keyword .kTo →
      Derives stopNode (.expr stop) →
      KeywordNodeOption .kBy (fun c e => Derives c (.expr e)) stepNodes step →
      doTok.kind = .keyword .kDo →
      Derives bodyNode (.stmtList body) →
      endForTok.kind = .keyword .kEndFor →
      Derives (.node .forStatement span
          ([.token forTok, varNode, .token assignTok, startNode,
              .token toTok, stopNode] ++
            stepNodes ++ [.token doTok, bodyNode, .token endForTok]))
        (.stmt (.forLoop var start stop step body span))
  | whileStatement {span : Span} {whileTok doTok endWhileTok : Token}
      {condNode : Cst} {cond : Expr} {bodyNode : Cst} {body : StmtList} :
      whileTok.kind = .keyword .kWhile →
      Derives condNode (.expr cond) →
      doTok.kind = .keyword .kDo →
      Derives bodyNode (.stmtList body) →
      endWhileTok.kind = .keyword .kEndWhile →
      Derives (.node .whileStatement span
          [.token whileTok, condNode, .token doTok, bodyNode, .token endWhileTok])
        (.stmt (.whileLoop cond body span))
  | repeatStatement {span : Span} {repeatTok untilTok endRepeatTok : Token}
      {bodyNode : Cst} {body : StmtList} {condNode : Cst} {cond : Expr} :
      repeatTok.kind = .keyword .kRepeat →
      Derives bodyNode (.stmtList body) →
      untilTok.kind = .keyword .kUntil →
      Derives condNode (.expr cond) →
      endRepeatTok.kind = .keyword .kEndRepeat →
      Derives (.node .repeatStatement span
          [.token repeatTok, bodyNode, .token untilTok, condNode, .token endRepeatTok])
        (.stmt (.repeatLoop body cond span))

  -- Places ---------------------------------------------------------------
  | variable {span : Span} {pathNode : Cst} {name : Name} :
      Derives pathNode (.name name) →
      Derives (.node .variable span [pathNode]) (.place (.var name span))
  | place {span : Span} {varNode : Cst} {p : Place} :
      Derives varNode (.place p) →
      Derives (.node .place span [varNode]) (.place p)

  -- Expressions ----------------------------------------------------------
  | expression {span : Span} {node : Cst} {e : Expr} :
      Derives node (.expr e) →
      Derives (.node .expression span [node]) (.expr e)
  | orExpression {span : Span} {first : Cst} {tail : List Cst}
      {e out : Expr} :
      Derives first (.expr e) →
      DerivesChained (fun c x => Derives c (.expr x)) Token.orOp? tail e out →
      Derives (.node .orExpression span (first :: tail)) (.expr (Expr.withSpan span out))
  | xorExpression {span : Span} {first : Cst} {tail : List Cst}
      {e out : Expr} :
      Derives first (.expr e) →
      DerivesChained (fun c x => Derives c (.expr x)) Token.xorOp? tail e out →
      Derives (.node .xorExpression span (first :: tail)) (.expr (Expr.withSpan span out))
  | andExpression {span : Span} {first : Cst} {tail : List Cst}
      {e out : Expr} :
      Derives first (.expr e) →
      DerivesChained (fun c x => Derives c (.expr x)) Token.andOp? tail e out →
      Derives (.node .andExpression span (first :: tail)) (.expr (Expr.withSpan span out))
  | compareExpression {span : Span} {first : Cst} {tail : List Cst}
      {e out : Expr} :
      Derives first (.expr e) →
      DerivesChained (fun c x => Derives c (.expr x)) Token.compareOp? tail e out →
      Derives (.node .compareExpression span (first :: tail)) (.expr (Expr.withSpan span out))
  | equExpression {span : Span} {first : Cst} {tail : List Cst}
      {e out : Expr} :
      Derives first (.expr e) →
      DerivesChained (fun c x => Derives c (.expr x)) Token.equOp? tail e out →
      Derives (.node .equExpression span (first :: tail)) (.expr (Expr.withSpan span out))
  | addExpression {span : Span} {first : Cst} {tail : List Cst}
      {e out : Expr} :
      Derives first (.expr e) →
      DerivesChained (fun c x => Derives c (.expr x)) Token.addOp? tail e out →
      Derives (.node .addExpression span (first :: tail)) (.expr (Expr.withSpan span out))
  | term {span : Span} {first : Cst} {tail : List Cst}
      {e out : Expr} :
      Derives first (.expr e) →
      DerivesChained (fun c x => Derives c (.expr x)) Token.termOp? tail e out →
      Derives (.node .term span (first :: tail)) (.expr (Expr.withSpan span out))
  | powerExpression {span : Span} {first : Cst} {tail : List Cst}
      {e out : Expr} :
      Derives first (.expr e) →
      DerivesChained (fun c x => Derives c (.expr x)) Token.powerOp? tail e out →
      Derives (.node .powerExpression span (first :: tail)) (.expr (Expr.withSpan span out))
  | unaryOperator {span : Span} {opTok : Token} {op : UnaryOp}
      {operandNode : Cst} {operand : Expr} :
      Token.unaryOp? opTok = some op →
      Derives operandNode (.expr operand) →
      Derives (.node .unaryExpression span [.token opTok, operandNode])
        (.expr (.unary op operand span))
  | unaryOperand {span : Span} {operandNode : Cst} {operand : Expr} :
      Derives operandNode (.expr operand) →
      Derives (.node .unaryExpression span [operandNode]) (.expr operand)
  | postfixExpression {span : Span} {primaryNode : Cst} {e : Expr} :
      Derives primaryNode (.expr e) →
      Derives (.node .postfixExpression span [primaryNode]) (.expr e)
  | primaryLiteral {span : Span} {litNode : Cst} {lit : Literal} :
      Derives litNode (.literal lit) →
      Derives (.node .primaryExpression span [litNode]) (.expr (.literal lit span))
  | primaryVariableOrCall {span : Span} {callNode : Cst} {e : Expr} :
      Derives callNode (.expr e) →
      Derives (.node .primaryExpression span [callNode]) (.expr e)
  | primaryParen {span : Span} {parenNode : Cst} {e : Expr} :
      Derives parenNode (.expr e) →
      Derives (.node .primaryExpression span [parenNode]) (.expr e)
  | parenExpression {span : Span} {lp rp : Token} {eNode : Cst} {e : Expr} :
      lp.kind = .punct .lparen →
      Derives eNode (.expr e) →
      rp.kind = .punct .rparen →
      Derives (.node .parenExpression span [.token lp, eNode, .token rp])
        (.expr (.paren e span))
  | literalExpression {span : Span} {tok : Token} {lit : Literal} :
      Literal.ofToken? tok = some lit →
      Derives (.node .literalExpression span [.token tok]) (.literal lit)
  | constantExpression {span : Span} {eNode : Cst} {e : Expr} :
      Derives eNode (.expr e) →
      Derives (.node .constantExpression span [eNode]) (.expr e)
  | variableOrCallVariable {span : Span} {pathNode : Cst} {name : Name} :
      Derives pathNode (.name name) →
      Derives (.node .variableOrCall span [pathNode]) (.expr (.var name span))
  | variableOrCallCallEmpty {span : Span} {pathNode : Cst} {name : Name}
      {lp rp : Token} :
      Derives pathNode (.name name) →
      lp.kind = .punct .lparen →
      rp.kind = .punct .rparen →
      Derives (.node .variableOrCall span [pathNode, .token lp, .token rp])
        (.expr (.call name [] span))
  | variableOrCallCallArgs {span : Span} {pathNode : Cst} {name : Name}
      {lp : Token} {argsNode : Cst} {args : ArgumentList} {rp : Token} :
      Derives pathNode (.name name) →
      lp.kind = .punct .lparen →
      Derives argsNode (.argList args) →
      rp.kind = .punct .rparen →
      Derives (.node .variableOrCall span
          [pathNode, .token lp, argsNode, .token rp])
        (.expr (.call name args.args span))
  | callExpression {span : Span} {callNode : Cst} {e : Expr} :
      Derives callNode (.expr e) →
      Derives (.node .callExpression span [callNode]) (.expr e)
  | argumentList {span : Span} {children : List Cst} {args : List CallArg} :
      CommaSeparated (fun c a => Derives c (.callArg a)) children args →
      Derives (.node .argumentList span children) (.argList ⟨args, span⟩)
  | argumentPositional {span : Span} {eNode : Cst} {e : Expr} :
      Derives eNode (.expr e) →
      Derives (.node .argument span [eNode]) (.callArg ⟨none, e, span⟩)
  | argumentNamed {span : Span} {nameNode : Cst} {name : Name}
      {assignTok : Token} {eNode : Cst} {e : Expr} :
      Derives nameNode (.name name) →
      assignTok.kind = .punct .assign →
      Derives eNode (.expr e) →
      Derives (.node .argument span [nameNode, .token assignTok, eNode])
        (.callArg ⟨some name, e, span⟩)

namespace Derives

/-- `Derives` accepts only trivia-free CSTs: any trivia leaf is a contradiction.
This is convenient for later proofs and documents the relation's domain. -/
theorem no_trivia_root {c : Cst} {a : Ast} (h : Derives c a) : c.isTrivia = false := by
  cases h <;> rfl

/-- A derived source unit carries the source-file node's span. This is the
root of the AST-to-CST source-span mapping. -/
theorem unit_span_eq {c : Cst} {u : SourceUnit} (h : Derives c (.unit u)) :
    u.span = c.span := by
  cases h with
  | sourceFile _ _ => rfl

/-- A derived source unit's span is well formed when its CST is. -/
theorem unit_span_wellFormed {c : Cst} {u : SourceUnit} (h : Derives c (.unit u))
    (hw : c.WellFormedNodes) : u.span.WellFormed := by
  rw [unit_span_eq h]
  exact hw.span_wellFormed

/-- A derived source unit's span lies within the declared source. -/
theorem unit_span_bounded {c : Cst} {u : SourceUnit} {file : FileId} {sourceLength : Nat}
    (h : Derives c (.unit u)) (hw : c.WellFormed file sourceLength) :
    u.span.file = file ∧ u.span.start ≤ sourceLength ∧ u.span.stop ≤ sourceLength ∧
      u.span.WellFormed := by
  rw [unit_span_eq h]
  exact hw.span_bounds

end Derives

/-- The AST-to-CST mapping relation: some trivia-free projection of `c` derives
`a`. This is the relation Phase 4 uses for `parse_lossless`, `parse_sound`, and
`parse_complete`. -/
def MapsTo (c : Cst) (a : Ast) : Prop :=
  ∃ c', Cst.stripTrivia c = some c' ∧ Derives c' a

/-- The source-span mapping: `a` is derived from `c` and the AST root span is
the CST root span. -/
def MapsToWithSpan (c : Cst) (a : Ast) : Prop :=
  MapsTo c a ∧ a.span = c.span

end Stc.Grammar
