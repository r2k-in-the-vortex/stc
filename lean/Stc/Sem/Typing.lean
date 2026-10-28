/-
The core static semantics (P0.2).

This module defines:

* the typing context and declared environment;
* the declarative typing judgment `ExprHasType`, `CallHasType`,
  `StdFnHasType`, `StmtHasType`, and whole-unit `HasType`;
* the core conversion relation and its characterization against `Stc.Exact`;
* the algorithmic checker `checkCore` with a structured `Rejection`;
* `HirWellFormed` and the soundness, rejection-characterization, and
  well-formedness theorems.

The HIR accepted here is the lowered form: variables are resolved, implicit
conversions are explicit `Cast` nodes, and call arguments are in formal
parameter order. The checker therefore requires exact type equality for
assignment, argument passing, and initializers, and checks only that explicit
casts are legal conversions. Inserting implicit conversions is the front end's
job (Phase 2); this module does not duplicate it.

`FOR` is fully defined by the declarative judgment but is deliberately
over-rejected by the P0.2 checker. `core_rejection_characterized` records that
over-rejection explicitly; P0.6's interpreter consumes the declarative
judgment, not `checkCore`.
-/
import Stc.Hir

set_option linter.unusedVariables false

namespace Stc.Sem

open Stc.Hir
open Stc.Syntax

/-! ## Declared environments and typing contexts -/

/-- A resolved function signature. Parameters are positional. -/
structure Signature where
  /-- The canonical name, retained for diagnostics and the dump. -/
  name : String
  /-- Parameter types in declaration order. -/
  params : List Ty
  /-- The optional result type. -/
  result : Option Ty
  /-- The declaration span. -/
  span : Span
deriving Repr, DecidableEq

/-- The declared environment of a unit. It is derived from the declarations
themselves; `checkCore` returns it as evidence. -/
structure Env where
  /-- Global root types. -/
  globalTypes : List Ty
  /-- Function signatures. -/
  functions : List Signature
  /-- Program field types. -/
  programLocals : List Ty
deriving Repr, DecidableEq

/-- The typing context of one body. `params`, `locals`, and `result` are the
enclosing function's or program's declarations; `globals` and `functions` are
unit-wide. -/
structure Ctx where
  /-- Global root types. -/
  globals : List Ty
  /-- Function signatures. -/
  functions : List Signature
  /-- Parameter types. -/
  params : List Ty
  /-- Local types. -/
  locals : List Ty
  /-- The function result type, when the enclosing POU has one. -/
  result : Option Ty
deriving Repr, DecidableEq

namespace Ctx

/-- The type of a resolved variable, if it is bound in this context. -/
def lookup : Ctx → VarId → Option Ty
  | ctx, .param index => ctx.params[index]?
  | ctx, .local index => ctx.locals[index]?
  | ctx, .global index => ctx.globals[index]?
  | ctx, .result => ctx.result

end Ctx

/-- The declared environment of a unit. -/
def Env.ofUnit (p : HirUnit) : Env :=
  { globalTypes := p.globals.map (·.ty)
    functions := p.functions.map fun fn =>
      { name := fn.name, params := fn.params.map (·.ty), result := fn.resultType,
        span := fn.span }
    programLocals := p.program.locals.map (·.ty) }

/-- The context of a global initializer: globals and functions are visible,
parameters and locals are not. -/
def Ctx.ofGlobals (env : Env) : Ctx :=
  { globals := env.globalTypes, functions := env.functions,
    params := [], locals := [], result := none }

/-- The context of a function body. -/
def Ctx.ofFunction (env : Env) (fn : Function) : Ctx :=
  { globals := env.globalTypes, functions := env.functions,
    params := fn.params.map (·.ty), locals := fn.locals.map (·.ty),
    result := fn.resultType }

/-- The context of a program cycle body. -/
def Ctx.ofProgram (env : Env) (p : Program) : Ctx :=
  { globals := env.globalTypes, functions := env.functions,
    params := [], locals := p.locals.map (·.ty), result := none }

/-! ## Conversions -/

/-- The core explicit-conversion relation. It accepts exactly the conversions
the analyzer's `can_convert` accepts for the included scalar types:

* a type converts to itself;
* any integer layout converts to any integer layout;
* any bit string converts to any bit string;
* any character type converts to any character type;
* integers and bit strings convert to each other;
* `BOOL` converts to and from integers and bit strings.

Character/integer conversions are deliberately excluded: the analyzer does not
provide them for the core (`CHAR_TO_UCHAR` and friends are character-to-
character conversions). -/
def ConversionLegal (src dst : Ty) : Prop :=
  src = dst ∨
    (src.isInt = true ∧ dst.isInt = true) ∨
    (src.isBits = true ∧ dst.isBits = true) ∨
    (src.isChar = true ∧ dst.isChar = true) ∨
    (src.isInt = true ∧ dst.isBits = true) ∨
    (src.isBits = true ∧ dst.isInt = true) ∨
    (src.isBool = true ∧ (dst.isInt = true ∨ dst.isBits = true)) ∨
    ((src.isInt = true ∨ src.isBits = true) ∧ dst.isBool = true)

/-- The executable form of `ConversionLegal`, used by the checker. -/
def conversionLegalB (src dst : Ty) : Bool :=
  (src == dst) ||
    (src.isInt && dst.isInt) ||
    (src.isBits && dst.isBits) ||
    (src.isChar && dst.isChar) ||
    (src.isInt && dst.isBits) ||
    (src.isBits && dst.isInt) ||
    (src.isBool && (dst.isInt || dst.isBits)) ||
    ((src.isInt || src.isBits) && dst.isBool)

/-- `ConversionLegal` is decided by `conversionLegalB`. -/
theorem conversion_legal_iff (src dst : Ty) :
    ConversionLegal src dst ↔ conversionLegalB src dst = true := by
  cases src <;> cases dst <;>
    simp [ConversionLegal, conversionLegalB, Ty.isBool, Ty.isInt, Ty.isBits,
      Ty.isChar, beq_iff_eq]

/-- The core implicit (assignment) conversions. This relation is not used by
the HIR checker, which expects lowering to have inserted the cast, but it
records the source-level rule the lowering must realize. -/
def ImplicitConversion (dst src : Ty) : Prop :=
  src = dst ∨
    (src.isInt = true ∧ dst.isInt = true) ∨
    (src.isBits = true ∧ dst.isBits = true ∧ src.width ≤ dst.width)

/-- Every implicit core conversion is an explicit conversion. -/
theorem implicit_conversion_legal {dst src : Ty} (h : ImplicitConversion dst src) :
    ConversionLegal src dst := by
  rcases h with h | h | h
  · subst h; exact Or.inl rfl
  · exact Or.inr (Or.inl h)
  · exact Or.inr (Or.inr (Or.inl ⟨h.1, h.2.1⟩))

/-! ## Operator typing -/

/-- Whether a unary operator is defined at a type. `-` and `+` require an
integer; `NOT` requires `BOOL` or a bit string. -/
def unaryWellTyped (op : UnaryOp) (ty : Ty) : Bool :=
  match op with
  | .neg | .pos => ty.isInt
  | .not => ty.isBitwise

/-- Whether both operands of a binary operator are well typed at the given
types. Arithmetic is integer-only, bitwise operators accept `BOOL` or matching
bit strings, and comparisons require equal operand types. All operators are
monomorphic at the HIR layer; lowering has already inserted the conversions
that the source analyzer would add. -/
def binaryWellTyped (op : BinaryOp) (lhs rhs : Ty) : Bool :=
  match op with
  | .add | .sub | .mul | .div | .mod | .pow => lhs == rhs && lhs.isInt
  | .and | .or | .xor => lhs == rhs && lhs.isBitwise
  | .eq | .ne | .lt | .le | .gt | .ge => lhs == rhs

/-- The result type of a binary operation. Comparisons produce `BOOL`; all
other operators produce their (equal) operand type. -/
def binaryResultTy (op : BinaryOp) (lhs _rhs : Ty) : Ty :=
  if op.isComparison then .bool else lhs

/-- The result type of an exact standard function on a list of argument types,
or `none` when the arguments do not fit the function's signature. This is the
single shared specification of `StdFnHasType` and the checker.

* `ABS(x)`: integer.
* `SEL(G, IN0, IN1)`: `G` is `BOOL`, inputs share a type.
* `MIN`/`MAX(a, b)`: matching magnitude types.
* `LIMIT(MN, IN, MX)`: three matching magnitude types.
* `MUX(K, IN0, ..., INn)`: integer selector, inputs share a type. -/
def stdResult? : StdFn → List Ty → Option Ty
  | .abs, [ty] => if ty.isInt then some ty else none
  | .sel, [g, a, b] => if g.isBool && (a == b) then some a else none
  | .min, [a, b] => if a == b && a.isMagnitude then some a else none
  | .max, [a, b] => if a == b && a.isMagnitude then some a else none
  | .limit, [mn, x, mx] =>
      if mn == x && x == mx && x.isMagnitude then some x else none
  | .mux, k :: d :: rest =>
      if k.isInt && rest.all (fun ty => ty == d) then some d else none
  | _, _ => none

/-- Positional argument types match formal parameter types exactly. -/
def typesMatch : List Ty → List Ty → Bool
  | [], [] => true
  | a :: as, b :: bs => (a == b) && typesMatch as bs
  | _, _ => false

/-- `typesMatch` is list equality. -/
theorem typesMatch_eq_true_iff (actual formal : List Ty) :
    typesMatch actual formal = true ↔ actual = formal := by
  induction actual generalizing formal with
  | nil => cases formal <;> simp [typesMatch]
  | cons a as ih =>
      cases formal with
      | nil => simp [typesMatch]
      | cons b bs => simp [typesMatch, ih, beq_iff_eq]

/-! ## `CASE` label validity -/

/-- Every label bound is representable in the selector type. -/
abbrev LabelInRange (ty : Ty) (label : CaseLabel) : Prop :=
  label.inRangeB ty = true

/-- A range label is ordered. -/
abbrev LabelOrdered (label : CaseLabel) : Prop :=
  label.orderedB = true

/-- The declarative `CASE` label validity predicate: every bound fits the
selector type, ranges are ordered, labels are distinct, and distinct labels do
not overlap. -/
def CaseLabelsValid (ty : Ty) (labels : List CaseLabel) : Prop :=
  (∀ label ∈ labels, LabelInRange ty label) ∧
    (∀ label ∈ labels, LabelOrdered label) ∧
    labels.Nodup ∧
    (∀ first ∈ labels, ∀ second ∈ labels,
      first.overlap second = true → first = second)

/-- The reason a `CASE` label set is rejected. -/
inductive CaseLabelProblem where
  /-- A bound does not fit the selector type. -/
  | outOfRange (ty : Ty) (label : CaseLabel)
  /-- A range's lower bound exceeds its upper bound. -/
  | unordered (label : CaseLabel)
  /-- Two distinct label occurrences overlap. -/
  | overlap (first second : CaseLabel)
deriving Repr, DecidableEq

/-- Find a later label that overlaps `label`. -/
def findOverlapFrom (label : CaseLabel) : List CaseLabel → Option CaseLabelProblem
  | [] => none
  | other :: rest =>
      if label.overlap other then some (.overlap label other)
      else findOverlapFrom label rest

/-- Check labels in order, including pairs across selections. -/
def checkCaseLabelsFrom (ty : Ty) : List CaseLabel → Option CaseLabelProblem
  | [] => none
  | label :: rest =>
      if !label.inRangeB ty then some (.outOfRange ty label)
      else if !label.orderedB then some (.unordered label)
      else
        match findOverlapFrom label rest with
        | some problem => some problem
        | none => checkCaseLabelsFrom ty rest

/-- Check every `CASE` label in source order. -/
def checkCaseLabels (ty : Ty) (labels : List CaseLabel) : Option CaseLabelProblem :=
  checkCaseLabelsFrom ty labels

/-! ## The declarative typing judgment -/

/-- The pointwise list relation. Core Lean does not provide `List.Forall₂`,
so the judgment carries its own copy. -/
inductive Forall2 {α β : Type} (R : α → β → Prop) : List α → List β → Prop where
  /-- The empty relation. -/
  | nil : Forall2 R [] []
  /-- A related head and related tail. -/
  | cons {a b as bs} : R a b → Forall2 R as bs → Forall2 R (a :: as) (b :: bs)

mutual
  /-- Expression typing. The judgment synthesizes a unique type for an
  expression in a context. -/
  inductive ExprHasType (ctx : Ctx) : Expr → Ty → Prop where
    /-- A well-formed typed literal has its declared type. -/
    | literal {lit : Literal} {span : Span} (h : lit.WellFormed) :
        ExprHasType ctx (.literal lit span) lit.ty
    /-- A bound variable has its declared type. -/
    | var {id : VarId} {span : Span} {ty : Ty}
        (h : ctx.lookup id = some ty) :
        ExprHasType ctx (.var id span) ty
    /-- A unary operation preserves its operand's type. -/
    | unary {op : UnaryOp} {operand : Expr} {span : Span} {ty : Ty}
        (hop : unaryWellTyped op ty = true)
        (hoperand : ExprHasType ctx operand ty) :
        ExprHasType ctx (.unary op operand span) ty
    /-- A binary operation requires well-typed, matching operands. -/
    | binary {op : BinaryOp} {lhs rhs : Expr} {span : Span} {lhsTy rhsTy : Ty}
        (hop : binaryWellTyped op lhsTy rhsTy = true)
        (hlhs : ExprHasType ctx lhs lhsTy)
        (hrhs : ExprHasType ctx rhs rhsTy) :
        ExprHasType ctx (.binary op lhs rhs span) (binaryResultTy op lhsTy rhsTy)
    /-- A call expression has the callee's result type; the callee must have a
    result. -/
    | call {callee : Callee} {args : List Expr} {span : Span} {ty : Ty}
        (h : CallHasType ctx callee args (some ty)) :
        ExprHasType ctx (.call callee args span) ty
    /-- A legal explicit conversion to a core type. -/
    | cast {operand : Expr} {to : Ty} {kind : CastKind} {span : Span} {src : Ty}
        (hoperand : ExprHasType ctx operand src)
        (hconv : ConversionLegal src to)
        (hcore : to.isCore = true) :
        ExprHasType ctx (.cast operand to kind span) to

  /-- Call typing. The result is `none` for a function without a result type. -/
  inductive CallHasType (ctx : Ctx) : Callee → List Expr → Option Ty → Prop where
    /-- A direct user-function call: arguments match the parameters positionally
    and the call has the declared result. -/
    | function {index : Nat} {span : Span} {sig : Signature} {args : List Expr}
        (hsig : ctx.functions[index]? = some sig)
        (hargs : Forall2 (ExprHasType ctx) args sig.params) :
        CallHasType ctx (.function index span) args sig.result
    /-- An exact standard-function call. -/
    | std {fn : StdFn} {span : Span} {args : List Expr} {ty : Ty}
        (h : StdFnHasType ctx fn args ty) :
        CallHasType ctx (.std fn span) args (some ty)

  /-- Standard-function typing, tied to the shared `stdResult?` specification. -/
  inductive StdFnHasType (ctx : Ctx) : StdFn → List Expr → Ty → Prop where
    | mk {fn : StdFn} {args : List Expr} {argTys : List Ty} {ty : Ty}
        (hargs : Forall2 (ExprHasType ctx) args argTys)
        (hres : stdResult? fn argTys = some ty) :
        StdFnHasType ctx fn args ty
end

/-- Every label in the `CASE` selections, in source order. -/
def allCaseLabels : List CaseArm → List CaseLabel
  | [] => []
  | arm :: rest => arm.labels ++ allCaseLabels rest

/-- The body of an `IF` arm is smaller than the arm. -/
theorem IfArm.sizeOf_body_lt (arm : IfArm) : sizeOf arm.body < sizeOf arm := by
  cases arm with
  | mk cond body span =>
      simp [IfArm.body]
      simp +arith

/-- The body of a `CASE` arm is smaller than the arm. -/
theorem CaseArm.sizeOf_body_lt (arm : CaseArm) : sizeOf arm.body < sizeOf arm := by
  cases arm with
  | mk labels body span =>
      simp [CaseArm.body]
      simp +arith

/-- Statement typing. `depth` is the enclosing loop depth; `controlled` is the
stack of active `FOR` control variables. Bodies are lists of statements, so
the judgment uses an explicit `∀` over members rather than a separate
mutually defined list predicate. -/
inductive StmtHasType : Ctx → Nat → List VarId → Stmt → Prop where
  /-- Assignment requires a bound, non-controlled target and an exactly
  matching value type. -/
  | assign {ctx : Ctx} {depth : Nat} {controlled : List VarId}
      {place : Place} {value : Expr} {span : Span} {ty : Ty}
      (hnotControlled : place.id ∉ controlled)
      (hplace : ctx.lookup place.id = some ty)
      (hvalue : ExprHasType ctx value ty) :
      StmtHasType ctx depth controlled (.assign place value span)
  /-- A call statement discards an optional result. -/
  | call {ctx : Ctx} {depth : Nat} {controlled : List VarId}
      {callee : Callee} {args : List Expr} {callSpan : Span} {span : Span}
      {result : Option Ty} (h : CallHasType ctx callee args result) :
      StmtHasType ctx depth controlled (.call (.call callee args callSpan) span)
  /-- `RETURN` is always available in a function or cycle body. -/
  | return_ {ctx : Ctx} {depth : Nat} {controlled : List VarId} {span : Span} :
      StmtHasType ctx depth controlled (.return_ span)
  /-- `EXIT` requires an enclosing loop. -/
  | exit {ctx : Ctx} {depth : Nat} {controlled : List VarId} {span : Span}
      (hdepth : 0 < depth) :
      StmtHasType ctx depth controlled (.exit span)
  /-- `CONTINUE` requires an enclosing loop. -/
  | continue_ {ctx : Ctx} {depth : Nat} {controlled : List VarId} {span : Span}
      (hdepth : 0 < depth) :
      StmtHasType ctx depth controlled (.continue_ span)
  /-- The empty statement. -/
  | empty {ctx : Ctx} {depth : Nat} {controlled : List VarId} {span : Span} :
      StmtHasType ctx depth controlled (.empty span)
  /-- Every `IF`/`ELSIF` condition is `BOOL`; bodies stay at the same depth. -/
  | ifThenElse {ctx : Ctx} {depth : Nat} {controlled : List VarId}
      {arms : List IfArm} {elseBody : List Stmt} {span : Span}
      (harmsCond : ∀ arm ∈ arms, ExprHasType ctx arm.cond .bool)
      (harmsBody : ∀ arm ∈ arms,
        ∀ stmt ∈ arm.body, StmtHasType ctx depth controlled stmt)
      (helse : ∀ stmt ∈ elseBody, StmtHasType ctx depth controlled stmt) :
      StmtHasType ctx depth controlled (.ifThenElse arms elseBody span)
  /-- `CASE` requires an integer selector and valid labels. -/
  | case {ctx : Ctx} {depth : Nat} {controlled : List VarId}
      {selector : Expr} {arms : List CaseArm} {elseBody : List Stmt}
      {span : Span} {ty : Ty}
      (hselector : ExprHasType ctx selector ty)
      (hint : ty.isInt = true)
      (hlabels : CaseLabelsValid ty (allCaseLabels arms))
      (hbodies : ∀ arm ∈ arms,
        ∀ stmt ∈ arm.body, StmtHasType ctx depth controlled stmt)
      (helse : ∀ stmt ∈ elseBody, StmtHasType ctx depth controlled stmt) :
      StmtHasType ctx depth controlled (.case selector arms elseBody span)
  /-- `FOR` requires an integer control variable that is not already
  controlled and is not modified in the body; bounds and step match the
  control type, and a constant zero step is rejected. The body runs one loop
  deeper with the control variable protected. -/
  | forLoop {ctx : Ctx} {depth : Nat} {controlled : List VarId}
      {var : Place} {start stop : Expr} {step : Option Expr}
      {body : List Stmt} {span : Span} {ty : Ty}
      (hnotControlled : var.id ∉ controlled)
      (hvar : ctx.lookup var.id = some ty)
      (hint : ty.isInt = true)
      (hstart : ExprHasType ctx start ty)
      (hstop : ExprHasType ctx stop ty)
      (hstep : ∀ expr, step = some expr → ExprHasType ctx expr ty)
      (hzero : ∀ expr, step = some expr →
        ∀ value, Expr.constInt? expr = some value → value ≠ 0)
      (hbody : ∀ stmt ∈ body, StmtHasType ctx (depth + 1) (var.id :: controlled) stmt) :
      StmtHasType ctx depth controlled (.forLoop var start stop step body span)
  /-- `WHILE` requires a `BOOL` condition; the body runs one loop deeper. -/
  | whileLoop {ctx : Ctx} {depth : Nat} {controlled : List VarId}
      {cond : Expr} {body : List Stmt} {span : Span}
      (hcond : ExprHasType ctx cond .bool)
      (hbody : ∀ stmt ∈ body, StmtHasType ctx (depth + 1) controlled stmt) :
      StmtHasType ctx depth controlled (.whileLoop cond body span)
  /-- `REPEAT` requires a `BOOL` condition; the body runs one loop deeper. -/
  | repeatLoop {ctx : Ctx} {depth : Nat} {controlled : List VarId}
      {body : List Stmt} {cond : Expr} {span : Span}
      (hbody : ∀ stmt ∈ body, StmtHasType ctx (depth + 1) controlled stmt)
      (hcond : ExprHasType ctx cond .bool) :
      StmtHasType ctx depth controlled (.repeatLoop body cond span)

/-- A statement list is well typed when each statement is. -/
def StmtListHasType (ctx : Ctx) (depth : Nat) (controlled : List VarId)
    (stmts : List Stmt) : Prop :=
  ∀ stmt ∈ stmts, StmtHasType ctx depth controlled stmt

/-! ## Whole-unit typing and well-formedness -/

/-- A global's initializer has the global's declared type. -/
def GlobalTyped (env : Env) (g : Global) : Prop :=
  ∀ expr, g.init = some expr →
    ExprHasType (Ctx.ofGlobals env) expr g.ty

/-- A function's local initializers and body are well typed. -/
def FunctionTyped (env : Env) (fn : Function) : Prop :=
  (∀ loc ∈ fn.locals, ∀ expr, loc.init = some expr →
    ExprHasType (Ctx.ofFunction env fn) expr loc.ty) ∧
  StmtListHasType (Ctx.ofFunction env fn) 0 [] fn.body

/-- A program's field initializers and cycle body are well typed. -/
def ProgramTyped (env : Env) (p : Program) : Prop :=
  (∀ loc ∈ p.locals, ∀ expr, loc.init = some expr →
    ExprHasType (Ctx.ofProgram env p) expr loc.ty) ∧
  StmtListHasType (Ctx.ofProgram env p) 0 [] p.body

/-- Declaration-level well-formedness: every declared type is a frozen core
type. Expression- and statement-level type restrictions are part of the
typing judgment. -/
structure FunctionWellFormed (fn : Function) : Prop where
  /-- Every parameter has a core type. -/
  params : ∀ param ∈ fn.params, param.ty.isCore = true
  /-- Every local has a core type. -/
  locals : ∀ loc ∈ fn.locals, loc.ty.isCore = true
  /-- The result, when present, has a core type. -/
  result : ∀ ty, fn.resultType = some ty → ty.isCore = true

/-- Program field well-formedness. -/
structure ProgramWellFormed (p : Program) : Prop where
  /-- Every field has a core type. -/
  locals : ∀ loc ∈ p.locals, loc.ty.isCore = true

/-- HIR well-formedness: all declared types are core types. -/
structure HirWellFormed (p : HirUnit) : Prop where
  /-- Global root types are core types. -/
  globals : ∀ glob ∈ p.globals, glob.ty.isCore = true
  /-- Function declaration types are core types. -/
  functions : ∀ fn ∈ p.functions, FunctionWellFormed fn
  /-- Program field types are core types. -/
  program : ProgramWellFormed p.program

/-- The whole-unit declarative typing judgment. The environment is the
declaration-derived one, and every declaration and body is well formed and well
typed. -/
structure HasType (p : HirUnit) (env : Env) : Prop where
  /-- The checker's environment is derived from the declarations. -/
  env_eq : env = Env.ofUnit p
  /-- Declaration-level well-formedness. -/
  wellformed : HirWellFormed p
  /-- Global initializers are well typed. -/
  globals : ∀ glob ∈ p.globals, GlobalTyped env glob
  /-- Function bodies are well typed. -/
  functions : ∀ fn ∈ p.functions, FunctionTyped env fn
  /-- The program body is well typed. -/
  program : ProgramTyped env p.program

/-! ## Rejections -/

/-- Named expression typing rules. -/
inductive ExprRule where
  /-- A variable is not bound in its context. -/
  | unboundVariable
  /-- A literal's payload does not fit its type. -/
  | literalOutOfRange
  /-- A unary operator is not defined at the operand type. -/
  | unaryOperand
  /-- Binary operands have different types or an unsupported type. -/
  | binaryOperand
  /-- A call's arguments do not match the callee's signature. -/
  | callArguments
  /-- A call to a function without a result is used as an expression. -/
  | callResultMissing
  /-- A cast is not a legal conversion or targets a non-core type. -/
  | castIllegal
deriving Repr, DecidableEq

/-- Named statement typing rules. -/
inductive StmtRule where
  /-- Assignment to a `FOR` control variable. -/
  | controlledAssignment
  /-- Assignment to an unbound variable. -/
  | assignTarget
  /-- Assignment value type does not match the target. -/
  | assignType
  /-- A call statement is not a call expression. -/
  | callStatement
  /-- `EXIT` outside a loop. -/
  | exitOutsideLoop
  /-- `CONTINUE` outside a loop. -/
  | continueOutsideLoop
  /-- A `WHILE` condition is not `BOOL`. -/
  | whileCondition
  /-- A `REPEAT` condition is not `BOOL`. -/
  | repeatCondition
  /-- A `CASE` selector is not an integer. -/
  | caseSelector
  /-- `FOR` is not checked by the P0.2 checker; this is a documented
  over-rejection, not a typing-rule failure. -/
  | forUnsupported
deriving Repr, DecidableEq

/-- A structured rejection. It carries the rule name and the exact construct
and context that failed, so `core_rejection_characterized` can state a precise
failure predicate. -/
inductive Rejection where
  /-- An expression failed a named rule. -/
  | expr (rule : ExprRule) (ctx : Ctx) (expr : Expr) (span : Span)
  /-- A statement failed a named rule. -/
  | stmt (rule : StmtRule) (ctx : Ctx) (depth : Nat) (controlled : List VarId)
      (stmt : Stmt) (span : Span)
  /-- An `IF`/`ELSIF` condition is not `BOOL`. -/
  | ifCondition (ctx : Ctx) (arm : IfArm)
  /-- A `CASE` label set failed. -/
  | caseLabels (ty : Ty) (problem : CaseLabelProblem) (labels : List CaseLabel) (span : Span)
  /-- A declared type is outside the frozen core. -/
  | typeNotCore (ty : Ty) (span : Span)
  /-- An initializer's type does not match its declaration. -/
  | initType (ctx : Ctx) (declTy : Ty) (value : Expr) (span : Span)

/-! ## Rejection semantics -/

/-- The semantic failure named by an expression rejection. -/
def ExprRuleFails (ctx : Ctx) (rule : ExprRule) (expr : Expr) : Prop :=
  match rule with
  | .unboundVariable =>
      ∃ id span, expr = .var id span ∧ ctx.lookup id = none
  | .literalOutOfRange =>
      ∃ lit span, expr = .literal lit span ∧ ¬ lit.WellFormed
  | .unaryOperand =>
      ∃ op operand span ty, expr = .unary op operand span ∧
        ExprHasType ctx operand ty ∧ unaryWellTyped op ty = false
  | .binaryOperand =>
      ∃ op lhs rhs span lhsTy rhsTy, expr = .binary op lhs rhs span ∧
        ExprHasType ctx lhs lhsTy ∧ ExprHasType ctx rhs rhsTy ∧
        binaryWellTyped op lhsTy rhsTy = false
  | .callArguments =>
      ∃ callee args span, expr = .call callee args span ∧
        ∀ result, ¬ CallHasType ctx callee args result
  | .callResultMissing =>
      ∃ callee args span, expr = .call callee args span ∧
        CallHasType ctx callee args none
  | .castIllegal =>
      ∃ operand to kind span src, expr = .cast operand to kind span ∧
        ExprHasType ctx operand src ∧
        (conversionLegalB src to = false ∨ to.isCore = false)

/-- The semantic failure named by a statement rejection. -/
def StmtRuleFails (ctx : Ctx) (depth : Nat) (controlled : List VarId)
    (rule : StmtRule) (stmt : Stmt) : Prop :=
  match rule with
  | .controlledAssignment =>
      ∃ place value span, stmt = .assign place value span ∧ place.id ∈ controlled
  | .assignTarget =>
      ∃ place value span, stmt = .assign place value span ∧ ctx.lookup place.id = none
  | .assignType =>
      ∃ place value span ty, stmt = .assign place value span ∧
        ctx.lookup place.id = some ty ∧
        ∃ valueTy, ExprHasType ctx value valueTy ∧ valueTy ≠ ty
  | .callStatement =>
      ∃ call span, stmt = .call call span ∧
        ∀ callee args callSpan, call ≠ .call callee args callSpan
  | .exitOutsideLoop =>
      ∃ span, stmt = .exit span ∧ depth = 0
  | .continueOutsideLoop =>
      ∃ span, stmt = .continue_ span ∧ depth = 0
  | .whileCondition =>
      ∃ cond body span ty, stmt = .whileLoop cond body span ∧
        ExprHasType ctx cond ty ∧ ty ≠ .bool
  | .repeatCondition =>
      ∃ body cond span ty, stmt = .repeatLoop body cond span ∧
        ExprHasType ctx cond ty ∧ ty ≠ .bool
  | .caseSelector =>
      ∃ selector arms elseBody span ty, stmt = .case selector arms elseBody span ∧
        ExprHasType ctx selector ty ∧ ty.isInt = false
  | .forUnsupported =>
      ∃ var start stop step body span,
        stmt = .forLoop var start stop step body span

/-- The semantic failure named by any rejection. -/
def FailsRule : Rejection → Prop
  | .expr rule ctx expr _ => ExprRuleFails ctx rule expr
  | .stmt rule ctx depth controlled stmt _ =>
      StmtRuleFails ctx depth controlled rule stmt
  | .ifCondition ctx arm =>
      ∃ ty, ExprHasType ctx arm.cond ty ∧ ty ≠ .bool
  | .caseLabels ty problem labels _ =>
      checkCaseLabels ty labels = some problem
  | .typeNotCore ty _ => ty.isCore = false
  | .initType ctx declTy value _ =>
      ∃ ty, ExprHasType ctx value ty ∧ ty ≠ declTy

/-! ## The core checker -/

mutual
  /-- Infer an expression's type. -/
  def inferExpr (ctx : Ctx) : Expr → Except Rejection Ty
    | .literal lit span =>
        if lit.WellFormed then .ok lit.ty
        else .error (.expr .literalOutOfRange ctx (.literal lit span) span)
    | .var id span =>
        match ctx.lookup id with
        | some ty => .ok ty
        | none => .error (.expr .unboundVariable ctx (.var id span) span)
    | .unary op operand span =>
        match inferExpr ctx operand with
        | .error err => .error err
        | .ok ty =>
            if unaryWellTyped op ty then .ok ty
            else .error (.expr .unaryOperand ctx (.unary op operand span) span)
    | .binary op lhs rhs span =>
        match inferExpr ctx lhs with
        | .error err => .error err
        | .ok lhsTy =>
            match inferExpr ctx rhs with
            | .error err => .error err
            | .ok rhsTy =>
                if binaryWellTyped op lhsTy rhsTy then
                  .ok (binaryResultTy op lhsTy rhsTy)
                else .error (.expr .binaryOperand ctx (.binary op lhs rhs span) span)
    | .call callee args span =>
        match inferArgs ctx args with
        | .error err => .error err
        | .ok argTys =>
            match callee with
            | .function index _ =>
                match ctx.functions[index]? with
                | none =>
                    .error (.expr .callArguments ctx (.call callee args span) span)
                | some sig =>
                    if typesMatch argTys sig.params then
                      match sig.result with
                      | some ty => .ok ty
                      | none =>
                          .error (.expr .callResultMissing ctx (.call callee args span) span)
                    else
                      .error (.expr .callArguments ctx (.call callee args span) span)
            | .std fn _ =>
                match stdResult? fn argTys with
                | some ty => .ok ty
                | none =>
                    .error (.expr .callArguments ctx (.call callee args span) span)
    | .cast operand to kind span =>
        match inferExpr ctx operand with
        | .error err => .error err
        | .ok src =>
            if conversionLegalB src to && to.isCore then .ok to
            else .error (.expr .castIllegal ctx (.cast operand to kind span) span)
  termination_by expr => sizeOf expr

  /-- Infer the types of an argument list in order. -/
  def inferArgs (ctx : Ctx) : List Expr → Except Rejection (List Ty)
    | [] => .ok []
    | arg :: rest =>
        match inferExpr ctx arg with
        | .error err => .error err
        | .ok ty =>
            match inferArgs ctx rest with
            | .error err => .error err
            | .ok tys => .ok (ty :: tys)
  termination_by args => sizeOf args
end

/-- Infer a call's result type. This is not recursive: the argument types are
already inferred by `inferArgs`. A call statement uses it directly so that a
function without a result is accepted as a statement. -/
def inferCall (ctx : Ctx) (callee : Callee) (args : List Expr) (span : Span) :
    Except Rejection (Option Ty) :=
  match inferArgs ctx args with
  | .error err => .error err
  | .ok argTys =>
      let call := Expr.call callee args span
      match callee with
      | .function index _ =>
          match ctx.functions[index]? with
          | none => .error (.expr .callArguments ctx call span)
          | some sig =>
              if typesMatch argTys sig.params then .ok sig.result
              else .error (.expr .callArguments ctx call span)
      | .std fn _ =>
          match stdResult? fn argTys with
          | some ty => .ok (some ty)
          | none => .error (.expr .callArguments ctx call span)

mutual
  /-- Check one statement. -/
  def checkStmt (ctx : Ctx) (depth : Nat) (controlled : List VarId) :
      Stmt → Except Rejection Unit
    | .assign place value span =>
        if h : place.id ∈ controlled then
          .error (.stmt .controlledAssignment ctx depth controlled
            (.assign place value span) span)
        else
          match ctx.lookup place.id with
          | none =>
              .error (.stmt .assignTarget ctx depth controlled
                (.assign place value span) span)
          | some ty =>
              match inferExpr ctx value with
              | .error err => .error err
              | .ok valueTy =>
                  if valueTy = ty then .ok ()
                  else .error (.stmt .assignType ctx depth controlled
                    (.assign place value span) span)
    | .call call span =>
        match call with
        | .call callee args callSpan =>
            match inferCall ctx callee args callSpan with
            | .error err => .error err
            | .ok _ => .ok ()
        | _ =>
            .error (.stmt .callStatement ctx depth controlled (.call call span) span)
    | .return_ _ => .ok ()
    | .exit span =>
        if 0 < depth then .ok ()
        else .error (.stmt .exitOutsideLoop ctx depth controlled (.exit span) span)
    | .continue_ span =>
        if 0 < depth then .ok ()
        else .error (.stmt .continueOutsideLoop ctx depth controlled (.continue_ span) span)
    | .empty _ => .ok ()
    | .ifThenElse arms elseBody span =>
        match checkIfArms ctx depth controlled arms with
        | .error err => .error err
        | .ok _ => checkStmts ctx depth controlled elseBody
    | .case selector arms elseBody span =>
        match inferExpr ctx selector with
        | .error err => .error err
        | .ok ty =>
            if ty.isInt then
              let labels := allCaseLabels arms
              match checkCaseLabels ty labels with
              | some problem => .error (.caseLabels ty problem labels span)
              | none =>
                  match checkCaseArms ctx depth controlled arms with
                  | .error err => .error err
                  | .ok _ =>
                      match checkStmts ctx depth controlled elseBody with
                      | .error err => .error err
                      | .ok _ => .ok ()
            else
              .error (.stmt .caseSelector ctx depth controlled
                (.case selector arms elseBody span) span)
    | .forLoop var start stop step body span =>
        -- P0.2 checks every other core statement; `FOR` is deliberately
        -- over-rejected by the checker and remains fully defined in the
        -- declarative judgment. P0.6's interpreter consumes the judgment.
        .error (.stmt .forUnsupported ctx depth controlled
          (.forLoop var start stop step body span) span)
    | .whileLoop cond body span =>
        match inferExpr ctx cond with
        | .error err => .error err
        | .ok ty =>
            if ty = .bool then checkStmts ctx (depth + 1) controlled body
            else
              .error (.stmt .whileCondition ctx depth controlled
                (.whileLoop cond body span) span)
    | .repeatLoop body cond span =>
        match inferExpr ctx cond with
        | .error err => .error err
        | .ok ty =>
            if ty = .bool then checkStmts ctx (depth + 1) controlled body
            else
              .error (.stmt .repeatCondition ctx depth controlled
                (.repeatLoop body cond span) span)
  termination_by stmt => sizeOf stmt

  /-- Check a statement list. -/
  def checkStmts (ctx : Ctx) (depth : Nat) (controlled : List VarId) :
      List Stmt → Except Rejection Unit
    | [] => .ok ()
    | stmt :: rest =>
        match checkStmt ctx depth controlled stmt with
        | .error err => .error err
        | .ok _ =>
            match checkStmts ctx depth controlled rest with
            | .error err => .error err
            | .ok _ => .ok ()
  termination_by stmts => sizeOf stmts

  /-- Check the arms of an `IF`. -/
  def checkIfArms (ctx : Ctx) (depth : Nat) (controlled : List VarId) :
      List IfArm → Except Rejection Unit
    | [] => .ok ()
    | arm :: rest =>
        match inferExpr ctx arm.cond with
        | .error err => .error err
        | .ok ty =>
            if ty = .bool then
              match checkStmts ctx depth controlled arm.body with
              | .error err => .error err
              | .ok _ =>
                  match checkIfArms ctx depth controlled rest with
                  | .error err => .error err
                  | .ok _ => .ok ()
            else .error (.ifCondition ctx arm)
  termination_by arms => sizeOf arms
  decreasing_by
    simp_wf
    all_goals
      first
      | (have h := IfArm.sizeOf_body_lt arm; omega)
      | (simp +arith)

  /-- Check the bodies of `CASE` selections. -/
  def checkCaseArms (ctx : Ctx) (depth : Nat) (controlled : List VarId) :
      List CaseArm → Except Rejection Unit
    | [] => .ok ()
    | arm :: rest =>
        match checkStmts ctx depth controlled arm.body with
        | .error err => .error err
        | .ok _ =>
            match checkCaseArms ctx depth controlled rest with
            | .error err => .error err
            | .ok _ => .ok ()
  termination_by arms => sizeOf arms
  decreasing_by
    simp_wf
    all_goals
      first
      | (have h := CaseArm.sizeOf_body_lt arm; omega)
      | (simp +arith)
end

/-! ## Declaration checking -/

/-- Check that a declared type is a core type. -/
def checkType (ty : Ty) (span : Span) : Except Rejection Unit :=
  if ty.isCore then .ok () else .error (.typeNotCore ty span)

/-- Check parameter types. -/
def checkParamTypes : List Param → Except Rejection Unit
  | [] => .ok ()
  | param :: rest =>
      match checkType param.ty param.span with
      | .error err => .error err
      | .ok _ => checkParamTypes rest

/-- Check local declared types. -/
def checkLocalTypes : List Local → Except Rejection Unit
  | [] => .ok ()
  | loc :: rest =>
      match checkType loc.ty loc.span with
      | .error err => .error err
      | .ok _ => checkLocalTypes rest

/-- Check global declared types. -/
def checkGlobalTypes : List Global → Except Rejection Unit
  | [] => .ok ()
  | glob :: rest =>
      match checkType glob.ty glob.span with
      | .error err => .error err
      | .ok _ => checkGlobalTypes rest

/-- Check a local initializer against its declared type. -/
def checkLocalInit (ctx : Ctx) (loc : Local) : Except Rejection Unit :=
  match loc.init with
  | none => .ok ()
  | some expr =>
      match inferExpr ctx expr with
      | .error err => .error err
      | .ok ty =>
          if ty = loc.ty then .ok ()
          else .error (.initType ctx loc.ty expr loc.span)

/-- Check every local initializer. -/
def checkLocalInits (ctx : Ctx) : List Local → Except Rejection Unit
  | [] => .ok ()
  | loc :: rest =>
      match checkLocalInit ctx loc with
      | .error err => .error err
      | .ok _ => checkLocalInits ctx rest

/-- Check a global initializer against its declared type. -/
def checkGlobalInit (ctx : Ctx) (glob : Global) : Except Rejection Unit :=
  match glob.init with
  | none => .ok ()
  | some expr =>
      match inferExpr ctx expr with
      | .error err => .error err
      | .ok ty =>
          if ty = glob.ty then .ok ()
          else .error (.initType ctx glob.ty expr glob.span)

/-- Check every global initializer. -/
def checkGlobalInits (ctx : Ctx) : List Global → Except Rejection Unit
  | [] => .ok ()
  | glob :: rest =>
      match checkGlobalInit ctx glob with
      | .error err => .error err
      | .ok _ => checkGlobalInits ctx rest

/-- The declaration-type checks after the result type. -/
def checkFunctionCoreTail (fn : Function) : Except Rejection Unit :=
  match checkParamTypes fn.params with
  | .error err => .error err
  | .ok _ => checkLocalTypes fn.locals

/-- Check a function's declaration types. -/
def checkFunctionCore (fn : Function) : Except Rejection Unit :=
  match fn.resultType with
  | some ty =>
      match checkType ty fn.span with
      | .error err => .error err
      | .ok _ => checkFunctionCoreTail fn
  | none => checkFunctionCoreTail fn

/-- Check every function's declaration types. -/
def checkFunctionsCore : List Function → Except Rejection Unit
  | [] => .ok ()
  | fn :: rest =>
      match checkFunctionCore fn with
      | .error err => .error err
      | .ok _ => checkFunctionsCore rest

/-- Check a program's declaration types. -/
def checkProgramCore (p : Program) : Except Rejection Unit :=
  checkLocalTypes p.locals

/-- Check a function's initializers and body. -/
def checkFunction (env : Env) (fn : Function) : Except Rejection Unit :=
  let ctx := Ctx.ofFunction env fn
  match checkLocalInits ctx fn.locals with
  | .error err => .error err
  | .ok _ => checkStmts ctx 0 [] fn.body

/-- Check every function body. -/
def checkFunctions (env : Env) : List Function → Except Rejection Unit
  | [] => .ok ()
  | fn :: rest =>
      match checkFunction env fn with
      | .error err => .error err
      | .ok _ => checkFunctions env rest

/-- Check a program's initializers and cycle body. -/
def checkProgram (env : Env) (p : Program) : Except Rejection Unit :=
  let ctx := Ctx.ofProgram env p
  match checkLocalInits ctx p.locals with
  | .error err => .error err
  | .ok _ => checkStmts ctx 0 [] p.body

/-- The core algorithmic checker. It checks declaration types, initializers,
and every body, and returns the declaration-derived environment. -/
def checkCore (p : HirUnit) : Except Rejection Env :=
  let env := Env.ofUnit p
  match checkGlobalTypes p.globals with
  | .error err => .error err
  | .ok _ =>
    match checkFunctionsCore p.functions with
    | .error err => .error err
    | .ok _ =>
      match checkProgramCore p.program with
      | .error err => .error err
      | .ok _ =>
        match checkGlobalInits (Ctx.ofGlobals env) p.globals with
        | .error err => .error err
        | .ok _ =>
          match checkFunctions env p.functions with
          | .error err => .error err
          | .ok _ =>
            match checkProgram env p.program with
            | .error err => .error err
            | .ok _ => .ok env

mutual
  theorem inferExpr_sound : ∀ (ctx : Ctx) (expr : Expr) (ty : Ty),
      inferExpr ctx expr = .ok ty → ExprHasType ctx expr ty
    | ctx, .literal lit span, ty, h => by
        simp only [inferExpr] at h
        split at h
        · next hok => cases h; exact .literal hok
        · cases h
    | ctx, .var id span, ty, h => by
        simp only [inferExpr] at h
        split at h
        · next ty' hlookup => cases h; exact .var hlookup
        · cases h
    | ctx, .unary op operand span, ty, h => by
        simp only [inferExpr] at h
        split at h
        · cases h
        · rename_i _x operandTy hoperand
          split at h
          · rename_i hok
            cases h
            exact .unary hok (inferExpr_sound ctx operand ty hoperand)
          · cases h
    | ctx, .binary op lhs rhs span, ty, h => by
        simp only [inferExpr] at h
        split at h
        · cases h
        · rename_i _x lhsTy hlhs
          split at h
          · cases h
          · rename_i _y rhsTy hrhs
            split at h
            · rename_i hok
              cases h
              exact .binary hok (inferExpr_sound ctx lhs lhsTy hlhs)
                (inferExpr_sound ctx rhs rhsTy hrhs)
            · cases h
    | ctx, .call (.function index cspan) args span, ty, h => by
        simp only [inferExpr] at h
        split at h
        · cases h
        · rename_i argTys hargs
          split at h
          · cases h
          · rename_i _y sig hsig
            split at h
            · rename_i hmatch
              split at h
              · rename_i _z result hres
                cases h
                exact .call (hres ▸ .function hsig (by
                  have hargs' := inferArgs_sound ctx args argTys hargs
                  have heq := (typesMatch_eq_true_iff argTys sig.params).mp hmatch
                  simpa [heq] using hargs'))
              · cases h
            · cases h
    | ctx, .call (.std fn cspan) args span, ty, h => by
        simp only [inferExpr] at h
        split at h
        · cases h
        · rename_i argTys hargs
          split at h
          · rename_i _x hres
            cases h
            exact .call (.std (.mk (inferArgs_sound ctx args argTys hargs) hres))
          · cases h
    | ctx, .cast operand to kind span, ty, h => by
        simp only [inferExpr] at h
        split at h
        · cases h
        · rename_i src hsrc
          split at h
          · rename_i hok
            simp only [Bool.and_eq_true] at hok
            cases h
            exact .cast (inferExpr_sound ctx operand src hsrc)
              ((conversion_legal_iff src to).mpr hok.1) hok.2
          · cases h
  termination_by ctx expr ty h => sizeOf expr
  decreasing_by
    simp_wf
    all_goals
      first
      | (exact List.sizeOf_lt_of_mem (by simp))
      | (simp +arith)

  theorem inferArgs_sound : ∀ (ctx : Ctx) (args : List Expr) (tys : List Ty),
      inferArgs ctx args = .ok tys → Forall2 (ExprHasType ctx) args tys
    | ctx, [], tys, h => by
        simp only [inferArgs] at h
        cases h
        exact .nil
    | ctx, arg :: rest, tys, h => by
        simp only [inferArgs] at h
        split at h
        · cases h
        · rename_i _x ty hty
          split at h
          · cases h
          · rename_i _y tys' htys
            cases h
            exact .cons (inferExpr_sound ctx arg ty hty)
              (inferArgs_sound ctx rest tys' htys)
  termination_by ctx args tys h => sizeOf args
  decreasing_by
    simp_wf
    all_goals
      first
      | (exact List.sizeOf_lt_of_mem (by simp))
      | (simp +arith)
end

/-- `inferCall` is sound for `CallHasType`. -/
theorem inferCall_sound :
    ∀ (ctx : Ctx) (callee : Callee) (args : List Expr) (span : Span) (result : Option Ty),
      inferCall ctx callee args span = .ok result → CallHasType ctx callee args result
  | ctx, .function index cspan, args, span, result, h => by
      simp only [inferCall] at h
      split at h
      · cases h
      · rename_i argTys hargs
        split at h
        · cases h
        · rename_i _y sig hsig
          split at h
          · rename_i hmatch
            have hargs' := inferArgs_sound ctx args argTys hargs
            have heq := (typesMatch_eq_true_iff argTys sig.params).mp hmatch
            subst heq
            cases h
            exact .function hsig hargs'
          · cases h
  | ctx, .std fn cspan, args, span, result, h => by
      simp only [inferCall] at h
      split at h
      · cases h
      · rename_i argTys hargs
        split at h
        · rename_i ty hres
          cases h
          exact .std (.mk (inferArgs_sound ctx args argTys hargs) hres)
        · cases h

/-- An ordered label overlaps itself. -/
theorem CaseLabel.overlap_self_of_ordered {label : CaseLabel}
    (h : label.orderedB = true) : label.overlap label = true := by
  apply label.overlap_self
  intro high hhigh
  simp [CaseLabel.orderedB, hhigh] at h
  exact h

/-- A successful scan finds no later label overlapping the head. -/
theorem findOverlapFrom_none {label : CaseLabel} {rest : List CaseLabel}
    (h : findOverlapFrom label rest = none) :
    ∀ other ∈ rest, label.overlap other = false := by
  induction rest with
  | nil => intro other hmem; simp at hmem
  | cons head tail ih =>
      simp only [findOverlapFrom] at h
      cases hov : label.overlap head with
      | true => simp [hov] at h
      | false =>
          simp [hov] at h
          intro other hmem
          simp only [List.mem_cons] at hmem
          rcases hmem with rfl | hmem
          · simpa using hov
          · exact ih h other hmem

/-- A successful label check satisfies the declarative `CASE` label validity
predicate. -/
theorem checkCaseLabelsFrom_none {ty : Ty} {labels : List CaseLabel}
    (h : checkCaseLabelsFrom ty labels = none) : CaseLabelsValid ty labels := by
  induction labels with
  | nil =>
      refine ⟨?_, ?_, ?_, ?_⟩ <;> simp
  | cons label rest ih =>
      simp only [checkCaseLabelsFrom] at h
      by_cases hin : label.inRangeB ty = true
      · by_cases hord : label.orderedB = true
        · cases hfind : findOverlapFrom label rest with
          | some problem => simp [hin, hord, hfind] at h
          | none =>
              simp [hin, hord, hfind] at h
              have ih' := ih h
              have hnone := findOverlapFrom_none hfind
              refine ⟨?_, ?_, ?_, ?_⟩
              · intro other hmem
                simp only [List.mem_cons] at hmem
                rcases hmem with hmem | hmem
                · subst hmem; exact hin
                · exact ih'.1 other hmem
              · intro other hmem
                simp only [List.mem_cons] at hmem
                rcases hmem with hmem | hmem
                · subst hmem; exact hord
                · exact ih'.2.1 other hmem
              · apply List.nodup_cons.mpr
                refine ⟨?_, ih'.2.2.1⟩
                intro hmem
                have hfalse := hnone label hmem
                rw [CaseLabel.overlap_self_of_ordered hord] at hfalse
                cases hfalse
              · intro first hfirst second hsecond hov
                simp only [List.mem_cons] at hfirst hsecond
                rcases hfirst with hfirst | hfirst
                · rcases hsecond with hsecond | hsecond
                  · subst first; subst second; rfl
                  · exfalso
                    rw [hfirst] at hov
                    have hfalse := hnone second hsecond
                    rw [hov] at hfalse
                    cases hfalse
                · rcases hsecond with hsecond | hsecond
                  · exfalso
                    rw [hsecond] at hov
                    have hfalse := hnone first hfirst
                    rw [CaseLabel.overlap_comm first label] at hov
                    rw [hov] at hfalse
                    cases hfalse
                  · exact ih'.2.2.2 first hfirst second hsecond hov
        · simp [hin, hord] at h
      · simp [hin] at h

/-- The label checker is sound for the declarative validity predicate. -/
theorem checkCaseLabels_sound {ty : Ty} {labels : List CaseLabel}
    (h : checkCaseLabels ty labels = none) : CaseLabelsValid ty labels :=
  checkCaseLabelsFrom_none h

mutual
  theorem checkStmt_sound : ∀ (ctx : Ctx) (depth : Nat) (controlled : List VarId)
      (stmt : Stmt), checkStmt ctx depth controlled stmt = .ok () →
      StmtHasType ctx depth controlled stmt
    | ctx, depth, controlled, .assign place value span, h => by
        simp only [checkStmt] at h
        split at h
        · simp at h
        · next hmem =>
          split at h
          · simp at h
          · next _ ty hlookup =>
            split at h
            · simp at h
            · next _ valueTy hinfer =>
              split at h
              · rename_i _ _ heq
                exact .assign hmem hlookup (by
                  have hs := inferExpr_sound ctx value valueTy hinfer
                  simpa [heq] using hs)
              · simp at h
    | ctx, depth, controlled, .call call span, h => by
        cases call with
        | call callee args callSpan =>
            simp only [checkStmt] at h
            split at h
            · simp at h
            · rename_i _ result hcall
              exact .call (inferCall_sound ctx callee args callSpan result hcall)
        | literal lit s => simp [checkStmt] at h
        | var id s => simp [checkStmt] at h
        | unary op operand s => simp [checkStmt] at h
        | binary op lhs rhs s => simp [checkStmt] at h
        | cast operand to kind s => simp [checkStmt] at h
    | ctx, depth, controlled, .return_ span, h => by
        simp only [checkStmt] at h
        exact .return_
    | ctx, depth, controlled, .exit span, h => by
        simp only [checkStmt] at h
        split at h
        · next hd => exact .exit hd
        · simp at h
    | ctx, depth, controlled, .continue_ span, h => by
        simp only [checkStmt] at h
        split at h
        · next hd => exact .continue_ hd
        · simp at h
    | ctx, depth, controlled, .empty span, h => by
        simp only [checkStmt] at h
        exact .empty
    | ctx, depth, controlled, .ifThenElse arms elseBody span, h => by
        simp only [checkStmt] at h
        split at h
        · simp at h
        · rename_i hcarms
          exact .ifThenElse
            (fun arm hmem => (checkIfArms_sound ctx depth controlled arms hcarms arm hmem).1)
            (fun arm hmem => (checkIfArms_sound ctx depth controlled arms hcarms arm hmem).2)
            (checkStmts_sound ctx depth controlled elseBody h)
    | ctx, depth, controlled, .case selector arms elseBody span, h => by
        simp only [checkStmt] at h
        split at h
        · simp at h
        · rename_i _ ty hsel
          split at h
          · rename_i hint
            split at h
            · simp at h
            · rename_i hlabels
              split at h
              · simp at h
              · rename_i _ hcarms
                split at h
                · simp at h
                · rename_i _ hbody
                  exact .case (inferExpr_sound ctx selector ty hsel) hint
                    (checkCaseLabels_sound hlabels)
                    (checkCaseArms_sound ctx depth controlled arms hcarms)
                    (checkStmts_sound ctx depth controlled elseBody hbody)
          · simp at h
    | ctx, depth, controlled, .forLoop var start stop step body span, h => by
        simp only [checkStmt] at h
        simp at h
    | ctx, depth, controlled, .whileLoop cond body span, h => by
        simp only [checkStmt] at h
        split at h
        · simp at h
        · rename_i _ ty hcond
          split at h
          · rename_i hbool
            exact .whileLoop
              (by have := inferExpr_sound ctx cond ty hcond; simpa [hbool] using this)
              (checkStmts_sound ctx (depth + 1) controlled body h)
          · simp at h
    | ctx, depth, controlled, .repeatLoop body cond span, h => by
        simp only [checkStmt] at h
        split at h
        · simp at h
        · rename_i _ ty hcond
          split at h
          · rename_i hbool
            exact .repeatLoop
              (checkStmts_sound ctx (depth + 1) controlled body h)
              (by have := inferExpr_sound ctx cond ty hcond; simpa [hbool] using this)
          · simp at h
  termination_by ctx depth controlled stmt _ => sizeOf stmt
  decreasing_by
    simp_wf
    all_goals
      first
      | (have h := IfArm.sizeOf_body_lt arm; omega)
      | (have h := CaseArm.sizeOf_body_lt arm; omega)
      | (simp +arith)

  theorem checkStmts_sound : ∀ (ctx : Ctx) (depth : Nat) (controlled : List VarId)
      (stmts : List Stmt), checkStmts ctx depth controlled stmts = .ok () →
      StmtListHasType ctx depth controlled stmts
    | ctx, depth, controlled, [], h => by
        simp only [checkStmts] at h
        intro stmt hmem
        simp at hmem
    | ctx, depth, controlled, head :: rest, h => by
        simp only [checkStmts] at h
        split at h
        · simp at h
        · rename_i _ hhead
          split at h
          · simp at h
          · rename_i _ hrest
            intro stmt hmem
            rcases List.mem_cons.mp hmem with hEq | hmem
            · exact hEq ▸ checkStmt_sound ctx depth controlled head hhead
            · exact checkStmts_sound ctx depth controlled rest hrest stmt hmem
  termination_by ctx depth controlled stmts _ => sizeOf stmts
  decreasing_by
    simp_wf
    all_goals
      first
      | (exact List.sizeOf_lt_of_mem (by simp))
      | (simp +arith)

  theorem checkIfArms_sound : ∀ (ctx : Ctx) (depth : Nat) (controlled : List VarId)
      (arms : List IfArm), checkIfArms ctx depth controlled arms = .ok () →
      ∀ arm ∈ arms, ExprHasType ctx arm.cond .bool ∧
        ∀ stmt ∈ arm.body, StmtHasType ctx depth controlled stmt
    | ctx, depth, controlled, [], h => by
        simp only [checkIfArms] at h
        intro arm hmem
        simp at hmem
    | ctx, depth, controlled, arm :: rest, h => by
        simp only [checkIfArms] at h
        split at h
        · simp at h
        · rename_i _ ty hcond
          split at h
          · rename_i hbool
            split at h
            · simp at h
            · rename_i _ hbody
              split at h
              · simp at h
              · rename_i _ hrest
                intro a hmem
                rcases List.mem_cons.mp hmem with hEq | hmem
                · refine hEq ▸ ⟨?_, ?_⟩
                  · have := inferExpr_sound ctx arm.cond ty hcond
                    simpa [hbool] using this
                  · exact checkStmts_sound ctx depth controlled arm.body hbody
                · exact checkIfArms_sound ctx depth controlled rest hrest a hmem
          · simp at h
  termination_by ctx depth controlled arms _ => sizeOf arms
  decreasing_by
    simp_wf
    all_goals
      first
      | (have h := IfArm.sizeOf_body_lt arm; omega)
      | (exact List.sizeOf_lt_of_mem (by simp))
      | (simp +arith)

  theorem checkCaseArms_sound : ∀ (ctx : Ctx) (depth : Nat) (controlled : List VarId)
      (arms : List CaseArm), checkCaseArms ctx depth controlled arms = .ok () →
      ∀ arm ∈ arms, ∀ stmt ∈ arm.body, StmtHasType ctx depth controlled stmt
    | ctx, depth, controlled, [], h => by
        simp only [checkCaseArms] at h
        intro arm hmem
        simp at hmem
    | ctx, depth, controlled, arm :: rest, h => by
        simp only [checkCaseArms] at h
        split at h
        · simp at h
        · rename_i _ hbody
          split at h
          · simp at h
          · rename_i _ hrest
            intro a hmem stmt hstmt
            rcases List.mem_cons.mp hmem with hEq | hmem
            · rw [hEq] at hstmt
              exact checkStmts_sound ctx depth controlled arm.body hbody stmt hstmt
            · exact checkCaseArms_sound ctx depth controlled rest hrest a hmem stmt hstmt
  termination_by ctx depth controlled arms _ => sizeOf arms
  decreasing_by
    simp_wf
    all_goals
      first
      | (have h := CaseArm.sizeOf_body_lt arm; omega)
      | (exact List.sizeOf_lt_of_mem (by simp))
      | (simp +arith)
end

/-! ## Declaration checker soundness -/

/-- `checkType` accepts only core types. -/
theorem checkType_sound {ty : Ty} {span : Span}
    (h : checkType ty span = .ok ()) : ty.isCore = true := by
  simp only [checkType] at h
  split at h
  · rename_i hcore
    exact hcore
  · cases h

/-- `checkParamTypes` accepts only core parameter types. -/
theorem checkParamTypes_sound :
    ∀ {params : List Param}, checkParamTypes params = .ok () →
      ∀ param ∈ params, param.ty.isCore = true
  | [], h => by
      simp only [checkParamTypes] at h
      intro param hmem
      simp at hmem
  | param :: rest, h => by
      simp only [checkParamTypes] at h
      split at h
      · cases h
      · rename_i hhead
        intro other hmem
        rcases List.mem_cons.mp hmem with hEq | hmem
        · exact hEq ▸ checkType_sound hhead
        · exact checkParamTypes_sound h other hmem

/-- `checkLocalTypes` accepts only core local types. -/
theorem checkLocalTypes_sound :
    ∀ {locals : List Local}, checkLocalTypes locals = .ok () →
      ∀ loc ∈ locals, loc.ty.isCore = true
  | [], h => by
      simp only [checkLocalTypes] at h
      intro loc hmem
      simp at hmem
  | loc :: rest, h => by
      simp only [checkLocalTypes] at h
      split at h
      · cases h
      · rename_i hhead
        intro other hmem
        rcases List.mem_cons.mp hmem with hEq | hmem
        · exact hEq ▸ checkType_sound hhead
        · exact checkLocalTypes_sound h other hmem

/-- `checkGlobalTypes` accepts only core global types. -/
theorem checkGlobalTypes_sound :
    ∀ {globals : List Global}, checkGlobalTypes globals = .ok () →
      ∀ glob ∈ globals, glob.ty.isCore = true
  | [], h => by
      simp only [checkGlobalTypes] at h
      intro glob hmem
      simp at hmem
  | glob :: rest, h => by
      simp only [checkGlobalTypes] at h
      split at h
      · cases h
      · rename_i hhead
        intro other hmem
        rcases List.mem_cons.mp hmem with hEq | hmem
        · exact hEq ▸ checkType_sound hhead
        · exact checkGlobalTypes_sound h other hmem

/-- `checkLocalInit` accepts only initializers with the declared type. -/
theorem checkLocalInit_sound {ctx : Ctx} {loc : Local}
    (h : checkLocalInit ctx loc = .ok ()) :
    ∀ expr, loc.init = some expr → ExprHasType ctx expr loc.ty := by
  intro expr hinit
  simp only [checkLocalInit, hinit] at h
  split at h
  · cases h
  · rename_i _ ty hinfer
    split at h
    · rename_i heq
      have hs := inferExpr_sound ctx expr ty hinfer
      simpa [heq] using hs
    · cases h

/-- `checkLocalInits` accepts only well-typed initializers. -/
theorem checkLocalInits_sound {ctx : Ctx} :
    ∀ {locals : List Local}, checkLocalInits ctx locals = .ok () →
      ∀ loc ∈ locals, ∀ expr, loc.init = some expr → ExprHasType ctx expr loc.ty
  | [], h => by
      simp only [checkLocalInits] at h
      intro loc hmem
      simp at hmem
  | loc :: rest, h => by
      simp only [checkLocalInits] at h
      split at h
      · cases h
      · rename_i hhead
        intro other hmem expr hinit
        rcases List.mem_cons.mp hmem with hEq | hmem
        · rw [hEq] at hinit ⊢
          exact checkLocalInit_sound hhead expr hinit
        · exact checkLocalInits_sound h other hmem expr hinit

/-- `checkGlobalInit` accepts only initializers with the declared type. -/
theorem checkGlobalInit_sound {ctx : Ctx} {glob : Global}
    (h : checkGlobalInit ctx glob = .ok ()) :
    ∀ expr, glob.init = some expr → ExprHasType ctx expr glob.ty := by
  intro expr hinit
  simp only [checkGlobalInit, hinit] at h
  split at h
  · cases h
  · rename_i _ ty hinfer
    split at h
    · rename_i heq
      have hs := inferExpr_sound ctx expr ty hinfer
      simpa [heq] using hs
    · cases h

/-- `checkGlobalInits` accepts only well-typed initializers. -/
theorem checkGlobalInits_sound {ctx : Ctx} :
    ∀ {globals : List Global}, checkGlobalInits ctx globals = .ok () →
      ∀ glob ∈ globals, ∀ expr, glob.init = some expr → ExprHasType ctx expr glob.ty
  | [], h => by
      simp only [checkGlobalInits] at h
      intro glob hmem
      simp at hmem
  | glob :: rest, h => by
      simp only [checkGlobalInits] at h
      split at h
      · cases h
      · rename_i hhead
        intro other hmem expr hinit
        rcases List.mem_cons.mp hmem with hEq | hmem
        · rw [hEq] at hinit ⊢
          exact checkGlobalInit_sound hhead expr hinit
        · exact checkGlobalInits_sound h other hmem expr hinit

/-- `checkFunctionCoreTail` accepts only core parameter and local types. -/
theorem checkFunctionCoreTail_sound {fn : Function}
    (h : checkFunctionCoreTail fn = .ok ()) :
    (∀ param ∈ fn.params, param.ty.isCore = true) ∧
      (∀ loc ∈ fn.locals, loc.ty.isCore = true) := by
  simp only [checkFunctionCoreTail] at h
  split at h
  · cases h
  · rename_i hparams
    exact ⟨checkParamTypes_sound hparams, checkLocalTypes_sound h⟩

/-- `checkFunctionCore` accepts only well-formed function declarations. -/
theorem checkFunctionCore_sound {fn : Function}
    (h : checkFunctionCore fn = .ok ()) : FunctionWellFormed fn := by
  simp only [checkFunctionCore] at h
  split at h
  · rename_i ty hresult
    split at h
    · cases h
    · rename_i hcheck
      have hcore := checkType_sound hcheck
      have htail' := checkFunctionCoreTail_sound h
      exact ⟨htail'.1, htail'.2, by intro t ht; rw [hresult] at ht; cases ht; exact hcore⟩
  · rename_i hresult
    have htail' := checkFunctionCoreTail_sound h
    exact ⟨htail'.1, htail'.2, by intro t ht; rw [hresult] at ht; cases ht⟩

/-- `checkFunctionsCore` accepts only well-formed function declarations. -/
theorem checkFunctionsCore_sound :
    ∀ {functions : List Function}, checkFunctionsCore functions = .ok () →
      ∀ fn ∈ functions, FunctionWellFormed fn
  | [], h => by
      simp only [checkFunctionsCore] at h
      intro fn hmem
      simp at hmem
  | fn :: rest, h => by
      simp only [checkFunctionsCore] at h
      split at h
      · cases h
      · rename_i hhead
        intro other hmem
        rcases List.mem_cons.mp hmem with hEq | hmem
        · exact hEq ▸ checkFunctionCore_sound hhead
        · exact checkFunctionsCore_sound h other hmem

/-- `checkProgramCore` accepts only core field types. -/
theorem checkProgramCore_sound {p : Program}
    (h : checkProgramCore p = .ok ()) : ProgramWellFormed p :=
  ⟨checkLocalTypes_sound h⟩

/-- `checkFunction` accepts only well-typed function bodies. -/
theorem checkFunction_sound {env : Env} {fn : Function}
    (h : checkFunction env fn = .ok ()) : FunctionTyped env fn := by
  simp only [checkFunction] at h
  split at h
  · cases h
  · rename_i hinits
    exact ⟨checkLocalInits_sound hinits,
      checkStmts_sound (Ctx.ofFunction env fn) 0 [] fn.body h⟩

/-- `checkFunctions` accepts only well-typed function bodies. -/
theorem checkFunctions_sound {env : Env} :
    ∀ {functions : List Function}, checkFunctions env functions = .ok () →
      ∀ fn ∈ functions, FunctionTyped env fn
  | [], h => by
      simp only [checkFunctions] at h
      intro fn hmem
      simp at hmem
  | fn :: rest, h => by
      simp only [checkFunctions] at h
      split at h
      · cases h
      · rename_i hhead
        intro other hmem
        rcases List.mem_cons.mp hmem with hEq | hmem
        · exact hEq ▸ checkFunction_sound hhead
        · exact checkFunctions_sound h other hmem

/-- `checkProgram` accepts only well-typed program bodies. -/
theorem checkProgram_sound {env : Env} {p : Program}
    (h : checkProgram env p = .ok ()) : ProgramTyped env p := by
  simp only [checkProgram] at h
  split at h
  · cases h
  · rename_i hinits
    exact ⟨checkLocalInits_sound hinits,
      checkStmts_sound (Ctx.ofProgram env p) 0 [] p.body h⟩

/-- `checkCore` is sound for the whole-unit declarative judgment. -/
theorem core_check_sound {p : HirUnit} {env : Env}
    (h : checkCore p = .ok env) : HasType p env := by
  simp only [checkCore] at h
  split at h
  · cases h
  · rename_i hglobalTypes
    split at h
    · cases h
    · rename_i hfunctionCores
      split at h
      · cases h
      · rename_i hprogramCore
        split at h
        · cases h
        · rename_i hglobalInits
          split at h
          · cases h
          · rename_i hfunctions
            split at h
            · cases h
            · rename_i hprogram
              cases h
              exact
                { env_eq := rfl
                  wellformed :=
                    { globals := checkGlobalTypes_sound hglobalTypes
                      functions := checkFunctionsCore_sound hfunctionCores
                      program := checkProgramCore_sound hprogramCore }
                  globals := checkGlobalInits_sound hglobalInits
                  functions := checkFunctions_sound hfunctions
                  program := checkProgram_sound hprogram }

/-- Accepted HIR is well formed. -/
theorem hir_wellformed {p : HirUnit} {env : Env}
    (h : checkCore p = .ok env) : HirWellFormed p :=
  (core_check_sound h).wellformed

/-! ## Uniqueness of typing -/

/-- The pointwise list relation is unique when its element relation is. -/
theorem Forall2.unique {α β : Type} {R : α → β → Prop} {as : List α} {bs cs : List β}
    (hR : ∀ a b c, R a b → R a c → b = c)
    (h1 : Forall2 R as bs) (h2 : Forall2 R as cs) : bs = cs := by
  induction h1 generalizing cs with
  | nil =>
      cases h2
      rfl
  | cons hab htail ih =>
      cases h2 with
      | cons hab' htail' =>
          rw [hR _ _ _ hab hab']
          rw [ih htail']

mutual
  /-- Expression typing is deterministic. -/
  theorem ExprHasType.unique : ∀ (ctx : Ctx) (expr : Expr) (T U : Ty),
      ExprHasType ctx expr T → ExprHasType ctx expr U → T = U
    | ctx, .literal lit span, T, U, h1, h2 => by
        cases h1; cases h2; rfl
    | ctx, .var id span, T, U, h1, h2 => by
        cases h1; cases h2
        rw [‹ctx.lookup id = some T›] at ‹ctx.lookup id = some U›
        exact Option.some.inj ‹some T = some U›
    | ctx, .unary op operand span, T, U, h1, h2 => by
        cases h1 with
        | unary hok hoperand =>
            cases h2 with
            | unary hok' hoperand' =>
                exact ExprHasType.unique ctx operand _ _ hoperand hoperand'
    | ctx, .binary op lhs rhs span, T, U, h1, h2 => by
        cases h1 with
        | binary hok hlhs hrhs =>
            cases h2 with
            | binary hok' hlhs' hrhs' =>
                have hlt : _ = _ := ExprHasType.unique ctx lhs _ _ hlhs hlhs'
                have hrt : _ = _ := ExprHasType.unique ctx rhs _ _ hrhs hrhs'
                simp only [binaryResultTy]
                by_cases hcmp : op.isComparison = true
                · simp [hcmp]
                · simp [hcmp]
                  exact hlt
    | ctx, .call callee args span, T, U, h1, h2 => by
        cases h1 with
        | call hc1 =>
            cases h2 with
            | call hc2 =>
                exact Option.some.inj
                  (CallHasType.unique ctx callee args (some T) (some U) hc1 hc2)
    | ctx, .cast operand to kind span, T, U, h1, h2 => by
        cases h1 with
        | cast _ _ _ =>
            cases h2 with
            | cast _ _ _ => rfl
  termination_by ctx expr T U h1 h2 => 10 * sizeOf expr + 4
  decreasing_by
    simp_wf
    all_goals
      first
      | (exact List.sizeOf_lt_of_mem (by simp))
      | (simp +arith)

  /-- Call typing is deterministic. -/
  theorem CallHasType.unique : ∀ (ctx : Ctx) (callee : Callee) (args : List Expr)
      (T U : Option Ty), CallHasType ctx callee args T → CallHasType ctx callee args U →
      T = U
    | ctx, .function index span, args, T, U, h1, h2 => by
        cases h1 with
        | function hsig hargs =>
            cases h2 with
            | function hsig' hargs' =>
                rw [hsig] at hsig'
                have hs : _ = _ := Option.some.inj hsig'
                subst hs
                rfl
    | ctx, .std fn span, args, T, U, h1, h2 => by
        cases h1 with
        | std hstd1 =>
            cases h2 with
            | std hstd2 =>
                rw [StdFnHasType.unique ctx fn args _ _ hstd1 hstd2]
  termination_by ctx callee args T U h1 h2 => 10 * sizeOf args + 3
  decreasing_by
    simp_wf
    all_goals omega

  /-- Standard-function typing is deterministic. -/
  theorem StdFnHasType.unique : ∀ (ctx : Ctx) (fn : StdFn) (args : List Expr)
      (T U : Ty), StdFnHasType ctx fn args T → StdFnHasType ctx fn args U → T = U
    | ctx, fn, args, T, U, h1, h2 => by
        cases h1 with
        | mk hargs hres =>
            cases h2 with
            | mk hargs' hres' =>
                have hargsEq : _ = _ := Forall2ExprUnique ctx hargs hargs'
                rw [hargsEq] at hres
                rw [hres] at hres'
                exact Option.some.inj hres'
  termination_by ctx fn args T U h1 h2 => 10 * sizeOf args + 2
  decreasing_by
    simp_wf
    all_goals omega

  /-- Pointwise uniqueness for argument type lists. -/
  theorem Forall2ExprUnique (ctx : Ctx) : ∀ {args : List Expr} {ts us : List Ty},
      Forall2 (ExprHasType ctx) args ts → Forall2 (ExprHasType ctx) args us → ts = us
    | [], ts, us, h1, h2 => by
        cases h1
        cases h2
        rfl
    | a :: as, ts, us, h1, h2 => by
        cases h1 with
        | cons ha htail =>
            cases h2 with
            | cons ha' htail' =>
                have hhead : _ = _ := ExprHasType.unique ctx a _ _ ha ha'
                subst hhead
                exact congrArg (List.cons _) (Forall2ExprUnique ctx htail htail')
  termination_by args ts us h1 h2 => 10 * sizeOf args + 1
  decreasing_by
    simp_wf
    all_goals
      first
      | (exact List.sizeOf_lt_of_mem (by simp))
      | (simp +arith)
end

/-! ## Rejection characterization -/

/-- A function call with an out-of-range callee index has no typing derivation. -/
theorem call_function_none {ctx : Ctx} {index : Nat} {span : Span} {args : List Expr}
    (h : ctx.functions[index]? = none) :
    ∀ result, ¬ CallHasType ctx (.function index span) args result := by
  intro result hcall
  cases hcall with
  | function hsig _ =>
      rw [hsig] at h
      cases h

/-- Argument types that do not match a function's formal parameters have no
typing derivation. -/
theorem call_function_mismatch {ctx : Ctx} {index : Nat} {span : Span}
    {args : List Expr} {sig : Signature} {argTys : List Ty}
    (hsig : ctx.functions[index]? = some sig)
    (hargs : inferArgs ctx args = .ok argTys)
    (hmismatch : typesMatch argTys sig.params = false) :
    ∀ result, ¬ CallHasType ctx (.function index span) args result := by
  intro result hcall
  cases hcall with
  | function hsig' hargs' =>
      rw [hsig] at hsig'
      have hs : sig = _ := Option.some.inj hsig'
      subst hs
      have huniq := Forall2ExprUnique ctx (inferArgs_sound ctx args argTys hargs) hargs'
      have htrue := (typesMatch_eq_true_iff argTys sig.params).mpr huniq
      rw [htrue] at hmismatch
      cases hmismatch

/-- Standard-function argument types outside the signature have no typing
derivation. -/
theorem call_std_none {ctx : Ctx} {fn : StdFn} {span : Span} {args : List Expr}
    {argTys : List Ty} (hargs : inferArgs ctx args = .ok argTys)
    (hres : stdResult? fn argTys = none) :
    ∀ result, ¬ CallHasType ctx (.std fn span) args result := by
  intro result hcall
  cases hcall with
  | std hstd =>
      cases hstd with
      | mk hargs' hres' =>
          have huniq := Forall2ExprUnique ctx (inferArgs_sound ctx args argTys hargs) hargs'
          rw [huniq] at hres
          rw [hres] at hres'
          cases hres'

mutual
  /-- Any expression rejection satisfies its named failure predicate. -/
  theorem inferExpr_error_fails : ∀ (ctx : Ctx) (expr : Expr) (r : Rejection),
      inferExpr ctx expr = .error r → FailsRule r
    | ctx, .literal lit span, r, h => by
        simp only [inferExpr] at h
        split at h
        · cases h
        · rename_i hnot
          cases h
          simp only [FailsRule]
          exact ⟨lit, span, rfl, hnot⟩
    | ctx, .var id span, r, h => by
        simp only [inferExpr] at h
        split at h
        · cases h
        · rename_i hnone
          cases h
          simp only [FailsRule]
          exact ⟨id, span, rfl, hnone⟩
    | ctx, .unary op operand span, r, h => by
        simp only [inferExpr] at h
        split at h
        · rename_i herr
          cases h
          exact inferExpr_error_fails ctx operand _ herr
        · rename_i _ ty hinfer
          split at h
          · cases h
          · rename_i hnot
            cases h
            simp only [FailsRule]
            exact ⟨op, operand, span, ty, rfl,
              inferExpr_sound ctx operand ty hinfer, by simpa using hnot⟩
    | ctx, .binary op lhs rhs span, r, h => by
        simp only [inferExpr] at h
        split at h
        · rename_i herr
          cases h
          exact inferExpr_error_fails ctx lhs _ herr
        · rename_i _ lhsTy hlhs
          split at h
          · rename_i herr
            cases h
            exact inferExpr_error_fails ctx rhs _ herr
          · rename_i _ rhsTy hrhs
            split at h
            · cases h
            · rename_i hnot
              cases h
              simp only [FailsRule]
              exact ⟨op, lhs, rhs, span, lhsTy, rhsTy, rfl,
                inferExpr_sound ctx lhs lhsTy hlhs,
                inferExpr_sound ctx rhs rhsTy hrhs, by simpa using hnot⟩
    | ctx, .call (.function index cspan) args span, r, h => by
        simp only [inferExpr] at h
        split at h
        · rename_i herr
          cases h
          exact inferArgs_error_fails ctx args _ herr
        · rename_i argTys hargs
          split at h
          · rename_i hnone
            cases h
            simp only [FailsRule]
            exact ⟨.function index cspan, args, span, rfl, call_function_none hnone⟩
          · rename_i _ sig hsig
            split at h
            · rename_i hmatch
              split at h
              · cases h
              · rename_i hnone
                cases h
                simp only [FailsRule]
                exact ⟨.function index cspan, args, span, rfl,
                  hnone ▸ (.function hsig (by
                    have hsound := inferArgs_sound ctx args argTys hargs
                    have huniq := (typesMatch_eq_true_iff argTys sig.params).mp hmatch
                    simpa [huniq] using hsound))⟩
            · rename_i hmismatch
              cases h
              simp only [FailsRule]
              exact ⟨.function index cspan, args, span, rfl,
                call_function_mismatch hsig hargs (by simpa using hmismatch)⟩
    | ctx, .call (.std fn cspan) args span, r, h => by
        simp only [inferExpr] at h
        split at h
        · rename_i herr
          cases h
          exact inferArgs_error_fails ctx args _ herr
        · rename_i argTys hargs
          split at h
          · cases h
          · rename_i hnone
            cases h
            simp only [FailsRule]
            exact ⟨.std fn cspan, args, span, rfl, call_std_none hargs hnone⟩
    | ctx, .cast operand to kind span, r, h => by
        simp only [inferExpr] at h
        split at h
        · rename_i herr
          cases h
          exact inferExpr_error_fails ctx operand _ herr
        · rename_i _ src hinfer
          split at h
          · cases h
          · rename_i hbad
            cases h
            simp only [Bool.and_eq_true] at hbad
            simp only [FailsRule]
            have hbad' : conversionLegalB src to = false ∨ to.isCore = false := by
              by_cases hc : conversionLegalB src to = true
              · right
                have hnot : ¬ to.isCore = true := by
                  intro hcore
                  exact hbad ⟨hc, hcore⟩
                simpa using hnot
              · left
                simpa using hc
            exact ⟨operand, to, kind, span, src, rfl,
              inferExpr_sound ctx operand src hinfer, hbad'⟩
  termination_by ctx expr r h => sizeOf expr
  decreasing_by
    simp_wf
    all_goals
      first
      | (exact List.sizeOf_lt_of_mem (by simp))
      | (simp +arith)

  /-- Any argument-list rejection satisfies its named failure predicate. -/
  theorem inferArgs_error_fails : ∀ (ctx : Ctx) (args : List Expr) (r : Rejection),
      inferArgs ctx args = .error r → FailsRule r
    | ctx, [], r, h => by
        simp only [inferArgs] at h
        cases h
    | ctx, arg :: rest, r, h => by
        simp only [inferArgs] at h
        split at h
        · rename_i herr
          cases h
          exact inferExpr_error_fails ctx arg _ herr
        · rename_i _ harg
          split at h
          · rename_i herr
            cases h
            exact inferArgs_error_fails ctx rest _ herr
          · cases h
  termination_by ctx args r h => sizeOf args
  decreasing_by
    simp_wf
    all_goals
      first
      | (exact List.sizeOf_lt_of_mem (by simp))
      | (simp +arith)
end

/-- Any `inferCall` rejection satisfies its named failure predicate. -/
theorem inferCall_error_fails {ctx : Ctx} {callee : Callee} {args : List Expr}
    {span : Span} {r : Rejection}
    (h : inferCall ctx callee args span = .error r) : FailsRule r := by
  simp only [inferCall] at h
  split at h
  · rename_i herr
    cases h
    exact inferArgs_error_fails ctx args _ herr
  · rename_i argTys hargs
    cases callee with
    | function index cspan =>
        cases hsig : ctx.functions[index]? with
        | none =>
            simp [hsig] at h
            cases h
            simp only [FailsRule]
            exact ⟨.function index cspan, args, span, rfl, call_function_none hsig⟩
        | some sig =>
            by_cases hmatch : typesMatch argTys sig.params = true
            · simp [hsig, hmatch] at h
            · simp [hsig, hmatch] at h
              cases h
              simp only [FailsRule]
              exact ⟨.function index cspan, args, span, rfl,
                call_function_mismatch hsig hargs (by simpa using hmatch)⟩
    | std fn cspan =>
        cases hres : stdResult? fn argTys with
        | some ty => simp [hres] at h
        | none =>
            simp [hres] at h
            cases h
            simp only [FailsRule]
            exact ⟨.std fn cspan, args, span, rfl, call_std_none hargs hres⟩

/-- Any `checkType` rejection satisfies its named failure predicate. -/
theorem checkType_error_fails {ty : Ty} {span : Span} {r : Rejection}
    (h : checkType ty span = .error r) : FailsRule r := by
  simp only [checkType] at h
  split at h
  · cases h
  · rename_i hnot
    cases h
    simp only [FailsRule]
    simpa using hnot

/-- Any `checkParamTypes` rejection satisfies its named failure predicate. -/
theorem checkParamTypes_error_fails :
    ∀ {params : List Param} {r : Rejection}, checkParamTypes params = .error r →
      FailsRule r
  | [], r, h => by
      simp only [checkParamTypes] at h
      cases h
  | param :: rest, r, h => by
      simp only [checkParamTypes] at h
      split at h
      · rename_i herr
        cases h
        exact checkType_error_fails herr
      · rename_i _ herrOK
        exact checkParamTypes_error_fails h

/-- Any `checkLocalTypes` rejection satisfies its named failure predicate. -/
theorem checkLocalTypes_error_fails :
    ∀ {locals : List Local} {r : Rejection}, checkLocalTypes locals = .error r →
      FailsRule r
  | [], r, h => by
      simp only [checkLocalTypes] at h
      cases h
  | loc :: rest, r, h => by
      simp only [checkLocalTypes] at h
      split at h
      · rename_i herr
        cases h
        exact checkType_error_fails herr
      · rename_i _ herrOK
        exact checkLocalTypes_error_fails h

/-- Any `checkGlobalTypes` rejection satisfies its named failure predicate. -/
theorem checkGlobalTypes_error_fails :
    ∀ {globals : List Global} {r : Rejection}, checkGlobalTypes globals = .error r →
      FailsRule r
  | [], r, h => by
      simp only [checkGlobalTypes] at h
      cases h
  | glob :: rest, r, h => by
      simp only [checkGlobalTypes] at h
      split at h
      · rename_i herr
        cases h
        exact checkType_error_fails herr
      · rename_i _ herrOK
        exact checkGlobalTypes_error_fails h

/-- Any `checkLocalInit` rejection satisfies its named failure predicate. -/
theorem checkLocalInit_error_fails {ctx : Ctx} {loc : Local} {r : Rejection}
    (h : checkLocalInit ctx loc = .error r) : FailsRule r := by
  cases hinit : loc.init with
  | none =>
      simp [checkLocalInit, hinit] at h
  | some expr =>
      simp only [checkLocalInit, hinit] at h
      split at h
      · rename_i herr
        cases h
        exact inferExpr_error_fails ctx expr _ herr
      · rename_i _ ty hinfer
        split at h
        · cases h
        · rename_i hne
          cases h
          simp only [FailsRule]
          exact ⟨ty, inferExpr_sound ctx expr ty hinfer, hne⟩

/-- Any `checkGlobalInit` rejection satisfies its named failure predicate. -/
theorem checkGlobalInit_error_fails {ctx : Ctx} {glob : Global} {r : Rejection}
    (h : checkGlobalInit ctx glob = .error r) : FailsRule r := by
  cases hinit : glob.init with
  | none =>
      simp [checkGlobalInit, hinit] at h
  | some expr =>
      simp only [checkGlobalInit, hinit] at h
      split at h
      · rename_i herr
        cases h
        exact inferExpr_error_fails ctx expr _ herr
      · rename_i _ ty hinfer
        split at h
        · cases h
        · rename_i hne
          cases h
          simp only [FailsRule]
          exact ⟨ty, inferExpr_sound ctx expr ty hinfer, hne⟩

/-- Any `checkLocalInits` rejection satisfies its named failure predicate. -/
theorem checkLocalInits_error_fails {ctx : Ctx} :
    ∀ {locals : List Local} {r : Rejection},
      checkLocalInits ctx locals = .error r → FailsRule r
  | [], r, h => by
      simp only [checkLocalInits] at h
      cases h
  | loc :: rest, r, h => by
      simp only [checkLocalInits] at h
      split at h
      · rename_i herr
        cases h
        exact checkLocalInit_error_fails herr
      · rename_i _ herrOK
        exact checkLocalInits_error_fails h

/-- Any `checkGlobalInits` rejection satisfies its named failure predicate. -/
theorem checkGlobalInits_error_fails {ctx : Ctx} :
    ∀ {globals : List Global} {r : Rejection},
      checkGlobalInits ctx globals = .error r → FailsRule r
  | [], r, h => by
      simp only [checkGlobalInits] at h
      cases h
  | glob :: rest, r, h => by
      simp only [checkGlobalInits] at h
      split at h
      · rename_i herr
        cases h
        exact checkGlobalInit_error_fails herr
      · rename_i _ herrOK
        exact checkGlobalInits_error_fails h

/-- Any `checkFunctionCoreTail` rejection satisfies its named failure predicate. -/
theorem checkFunctionCoreTail_error_fails {fn : Function} {r : Rejection}
    (h : checkFunctionCoreTail fn = .error r) : FailsRule r := by
  simp only [checkFunctionCoreTail] at h
  split at h
  · rename_i herr
    cases h
    exact checkParamTypes_error_fails herr
  · rename_i _ herrOK
    exact checkLocalTypes_error_fails h

/-- Any `checkFunctionCore` rejection satisfies its named failure predicate. -/
theorem checkFunctionCore_error_fails {fn : Function} {r : Rejection}
    (h : checkFunctionCore fn = .error r) : FailsRule r := by
  simp only [checkFunctionCore] at h
  split at h
  · rename_i _ herr
    split at h
    · rename_i herr'
      cases h
      exact checkType_error_fails herr'
    · rename_i _ herrOK
      exact checkFunctionCoreTail_error_fails h
  · rename_i _ herrOK
    exact checkFunctionCoreTail_error_fails h

/-- Any `checkFunctionsCore` rejection satisfies its named failure predicate. -/
theorem checkFunctionsCore_error_fails :
    ∀ {functions : List Function} {r : Rejection},
      checkFunctionsCore functions = .error r → FailsRule r
  | [], r, h => by
      simp only [checkFunctionsCore] at h
      cases h
  | fn :: rest, r, h => by
      simp only [checkFunctionsCore] at h
      split at h
      · rename_i herr
        cases h
        exact checkFunctionCore_error_fails herr
      · rename_i _ herrOK
        exact checkFunctionsCore_error_fails h

/-- Any `checkProgramCore` rejection satisfies its named failure predicate. -/
theorem checkProgramCore_error_fails {p : Program} {r : Rejection}
    (h : checkProgramCore p = .error r) : FailsRule r :=
  checkLocalTypes_error_fails h

mutual
  /-- Any statement rejection satisfies its named failure predicate. -/
  theorem checkStmt_error_fails : ∀ (ctx : Ctx) (depth : Nat) (controlled : List VarId)
      (stmt : Stmt) (r : Rejection),
      checkStmt ctx depth controlled stmt = .error r → FailsRule r
    | ctx, depth, controlled, .assign place value span, r, h => by
        simp only [checkStmt] at h
        split at h
        · rename_i hmem
          cases h
          simp only [FailsRule]
          exact ⟨place, value, span, rfl, hmem⟩
        · split at h
          · rename_i hnone
            cases h
            simp only [FailsRule]
            exact ⟨place, value, span, rfl, hnone⟩
          · rename_i _ ty hlookup
            split at h
            · rename_i herr
              cases h
              exact inferExpr_error_fails ctx value _ herr
            · rename_i _ valueTy hinfer
              split at h
              · cases h
              · rename_i hne
                cases h
                simp only [FailsRule]
                exact ⟨place, value, span, ty, rfl, hlookup,
                  ⟨valueTy, inferExpr_sound ctx value valueTy hinfer, by simpa using hne⟩⟩
    | ctx, depth, controlled, .call call span, r, h => by
        cases call with
        | call callee args callSpan =>
            simp only [checkStmt] at h
            split at h
            · rename_i herr
              cases h
              exact inferCall_error_fails herr
            · cases h
        | literal lit s =>
            simp only [checkStmt] at h
            cases h
            simp only [FailsRule]
            exact ⟨.literal lit s, span, rfl, by intro c a cs heq; cases heq⟩
        | var id s =>
            simp only [checkStmt] at h
            cases h
            simp only [FailsRule]
            exact ⟨.var id s, span, rfl, by intro c a cs heq; cases heq⟩
        | unary op operand s =>
            simp only [checkStmt] at h
            cases h
            simp only [FailsRule]
            exact ⟨.unary op operand s, span, rfl, by intro c a cs heq; cases heq⟩
        | binary op lhs rhs s =>
            simp only [checkStmt] at h
            cases h
            simp only [FailsRule]
            exact ⟨.binary op lhs rhs s, span, rfl, by intro c a cs heq; cases heq⟩
        | cast operand to kind s =>
            simp only [checkStmt] at h
            cases h
            simp only [FailsRule]
            exact ⟨.cast operand to kind s, span, rfl, by intro c a cs heq; cases heq⟩
    | ctx, depth, controlled, .return_ span, r, h => by
        simp only [checkStmt] at h
        cases h
    | ctx, depth, controlled, .exit span, r, h => by
        simp only [checkStmt] at h
        split at h
        · cases h
        · rename_i hd
          cases h
          simp only [FailsRule]
          exact ⟨span, rfl, by simpa using hd⟩
    | ctx, depth, controlled, .continue_ span, r, h => by
        simp only [checkStmt] at h
        split at h
        · cases h
        · rename_i hd
          cases h
          simp only [FailsRule]
          exact ⟨span, rfl, by simpa using hd⟩
    | ctx, depth, controlled, .empty span, r, h => by
        simp only [checkStmt] at h
        cases h
    | ctx, depth, controlled, .ifThenElse arms elseBody span, r, h => by
        simp only [checkStmt] at h
        split at h
        · rename_i herr
          cases h
          exact checkIfArms_error_fails ctx depth controlled arms _ herr
        · rename_i _ herrOK
          exact checkStmts_error_fails ctx depth controlled elseBody _ h
    | ctx, depth, controlled, .case selector arms elseBody span, r, h => by
        simp only [checkStmt] at h
        split at h
        · rename_i herr
          cases h
          exact inferExpr_error_fails ctx selector _ herr
        · rename_i _ ty hsel
          split at h
          · rename_i hint
            split at h
            · rename_i hproblem
              cases h
              simp only [FailsRule]
              exact hproblem
            · rename_i _ hcarms
              split at h
              · rename_i herr
                cases h
                exact checkCaseArms_error_fails ctx depth controlled arms _ herr
              · rename_i _ herrOK
                split at h
                · rename_i herr
                  cases h
                  exact checkStmts_error_fails ctx depth controlled elseBody _ herr
                · cases h
          · rename_i hnot
            cases h
            simp only [FailsRule]
            exact ⟨selector, arms, elseBody, span, ty, rfl,
              inferExpr_sound ctx selector ty hsel, by simpa using hnot⟩
    | ctx, depth, controlled, .forLoop var start stop step body span, r, h => by
        simp only [checkStmt] at h
        cases h
        simp only [FailsRule]
        exact ⟨var, start, stop, step, body, span, rfl⟩
    | ctx, depth, controlled, .whileLoop cond body span, r, h => by
        simp only [checkStmt] at h
        split at h
        · rename_i herr
          cases h
          exact inferExpr_error_fails ctx cond _ herr
        · rename_i _ ty hcond
          split at h
          · rename_i _ herrOK
            exact checkStmts_error_fails ctx (depth + 1) controlled body _ h
          · rename_i hnot
            cases h
            simp only [FailsRule]
            exact ⟨cond, body, span, ty, rfl,
              inferExpr_sound ctx cond ty hcond, by simpa using hnot⟩
    | ctx, depth, controlled, .repeatLoop body cond span, r, h => by
        simp only [checkStmt] at h
        split at h
        · rename_i herr
          cases h
          exact inferExpr_error_fails ctx cond _ herr
        · rename_i _ ty hcond
          split at h
          · rename_i _ herrOK
            exact checkStmts_error_fails ctx (depth + 1) controlled body _ h
          · rename_i hnot
            cases h
            simp only [FailsRule]
            exact ⟨body, cond, span, ty, rfl,
              inferExpr_sound ctx cond ty hcond, by simpa using hnot⟩
  termination_by ctx depth controlled stmt r h => sizeOf stmt
  decreasing_by
    simp_wf
    all_goals
      first
      | (have h := IfArm.sizeOf_body_lt arm; omega)
      | (have h := CaseArm.sizeOf_body_lt arm; omega)
      | (simp +arith)

  /-- Any statement-list rejection satisfies its named failure predicate. -/
  theorem checkStmts_error_fails : ∀ (ctx : Ctx) (depth : Nat) (controlled : List VarId)
      (stmts : List Stmt) (r : Rejection),
      checkStmts ctx depth controlled stmts = .error r → FailsRule r
    | ctx, depth, controlled, [], r, h => by
        simp only [checkStmts] at h
        cases h
    | ctx, depth, controlled, stmt :: rest, r, h => by
        simp only [checkStmts] at h
        split at h
        · rename_i herr
          cases h
          exact checkStmt_error_fails ctx depth controlled stmt _ herr
        · rename_i _ herrOK
          split at h
          · rename_i herr
            cases h
            exact checkStmts_error_fails ctx depth controlled rest _ herr
          · cases h
  termination_by ctx depth controlled stmts r h => sizeOf stmts
  decreasing_by
    simp_wf
    all_goals
      first
      | (exact List.sizeOf_lt_of_mem (by simp))
      | (simp +arith)

  /-- Any `IF`-arm rejection satisfies its named failure predicate. -/
  theorem checkIfArms_error_fails : ∀ (ctx : Ctx) (depth : Nat) (controlled : List VarId)
      (arms : List IfArm) (r : Rejection),
      checkIfArms ctx depth controlled arms = .error r → FailsRule r
    | ctx, depth, controlled, [], r, h => by
        simp only [checkIfArms] at h
        cases h
    | ctx, depth, controlled, arm :: rest, r, h => by
        simp only [checkIfArms] at h
        split at h
        · rename_i herr
          cases h
          exact inferExpr_error_fails ctx arm.cond _ herr
        · rename_i _ ty hcond
          split at h
          · rename_i hbool
            split at h
            · rename_i herr
              cases h
              exact checkStmts_error_fails ctx depth controlled arm.body _ herr
            · rename_i _ herrOK
              split at h
              · rename_i herr
                cases h
                exact checkIfArms_error_fails ctx depth controlled rest _ herr
              · cases h
          · rename_i hnot
            cases h
            simp only [FailsRule]
            exact ⟨ty, inferExpr_sound ctx arm.cond ty hcond, by simpa using hnot⟩
  termination_by ctx depth controlled arms r h => sizeOf arms
  decreasing_by
    simp_wf
    all_goals
      first
      | (have h := IfArm.sizeOf_body_lt arm; omega)
      | (exact List.sizeOf_lt_of_mem (by simp))
      | (simp +arith)

  /-- Any `CASE`-arm rejection satisfies its named failure predicate. -/
  theorem checkCaseArms_error_fails : ∀ (ctx : Ctx) (depth : Nat) (controlled : List VarId)
      (arms : List CaseArm) (r : Rejection),
      checkCaseArms ctx depth controlled arms = .error r → FailsRule r
    | ctx, depth, controlled, [], r, h => by
        simp only [checkCaseArms] at h
        cases h
    | ctx, depth, controlled, arm :: rest, r, h => by
        simp only [checkCaseArms] at h
        split at h
        · rename_i herr
          cases h
          exact checkStmts_error_fails ctx depth controlled arm.body _ herr
        · rename_i _ herrOK
          split at h
          · rename_i herr
            cases h
            exact checkCaseArms_error_fails ctx depth controlled rest _ herr
          · cases h
  termination_by ctx depth controlled arms r h => sizeOf arms
  decreasing_by
    simp_wf
    all_goals
      first
      | (have h := CaseArm.sizeOf_body_lt arm; omega)
      | (exact List.sizeOf_lt_of_mem (by simp))
      | (simp +arith)
end

/-- Any `checkFunction` rejection satisfies its named failure predicate. -/
theorem checkFunction_error_fails {env : Env} {fn : Function} {r : Rejection}
    (h : checkFunction env fn = .error r) : FailsRule r := by
  simp only [checkFunction] at h
  split at h
  · rename_i herr
    cases h
    exact checkLocalInits_error_fails herr
  · rename_i _ herrOK
    exact checkStmts_error_fails (Ctx.ofFunction env fn) 0 [] fn.body _ h

/-- Any `checkFunctions` rejection satisfies its named failure predicate. -/
theorem checkFunctions_error_fails {env : Env} :
    ∀ {functions : List Function} {r : Rejection},
      checkFunctions env functions = .error r → FailsRule r
  | [], r, h => by
      simp only [checkFunctions] at h
      cases h
  | fn :: rest, r, h => by
      simp only [checkFunctions] at h
      split at h
      · rename_i herr
        cases h
        exact checkFunction_error_fails herr
      · rename_i _ herrOK
        exact checkFunctions_error_fails h

/-- Any `checkProgram` rejection satisfies its named failure predicate. -/
theorem checkProgram_error_fails {env : Env} {p : Program} {r : Rejection}
    (h : checkProgram env p = .error r) : FailsRule r := by
  simp only [checkProgram] at h
  split at h
  · rename_i herr
    cases h
    exact checkLocalInits_error_fails herr
  · rename_i _ herrOK
    exact checkStmts_error_fails (Ctx.ofProgram env p) 0 [] p.body _ h

/-- Any `checkCore` rejection satisfies its named failure predicate. -/
theorem checkCore_error_fails {p : HirUnit} {r : Rejection}
    (h : checkCore p = .error r) : FailsRule r := by
  simp only [checkCore] at h
  split at h
  · rename_i herr
    cases h
    exact checkGlobalTypes_error_fails herr
  · rename_i _ herrOK
    split at h
    · rename_i herr
      cases h
      exact checkFunctionsCore_error_fails herr
    · rename_i _ herrOK
      split at h
      · rename_i herr
        cases h
        exact checkProgramCore_error_fails herr
      · rename_i _ herrOK
        split at h
        · rename_i herr
          cases h
          exact checkGlobalInits_error_fails herr
        · rename_i _ herrOK
          split at h
          · rename_i herr
            cases h
            exact checkFunctions_error_fails herr
          · rename_i _ herrOK
            split at h
            · rename_i herr
              cases h
              exact checkProgram_error_fails herr
            · cases h

/-- Every rejected core program fails a named rule. This is the rejection
characterization required in place of `core_check_complete`; `FOR` is the one
deliberate over-rejection and is named by `StmtRule.forUnsupported`. -/
theorem core_rejection_characterized {p : HirUnit} {r : Rejection}
    (h : checkCore p = .error r) : FailsRule r :=
  checkCore_error_fails h
