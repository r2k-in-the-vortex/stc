/-
The typed HIR syntax for the core subset (P0.2).

Expressions, statements, places, declarations, and programs are defined here.
The HIR is resolved and lowered: variables are indexed by `VarId`, callees are
resolved to function or standard-function targets, conversions are explicit
`Cast` nodes, and `CASE` labels are folded constant integers. The Rust dump in
`crates/stc-hir/src/dump.rs` is aligned with this family at P0.6, but the Lean
family is the semantic reference.

The family deliberately drops surface-only structure: parentheses, source
spellings of types, named call arguments (the dump emits formal-parameter
order), and constant-expression syntax in `CASE` labels. Those are front-end
concerns owned by the lowering phase.
-/
import Stc.Hir.Types

namespace Stc.Hir

open Stc.Syntax

/-- Unary operators in the core. -/
inductive UnaryOp where
  /-- Arithmetic negation. -/
  | neg
  /-- Arithmetic identity. -/
  | pos
  /-- Boolean or bitwise complement. -/
  | not
deriving Repr, DecidableEq

/-- Binary operators in the core. `and`/`or`/`xor` are bitwise on `BOOL` or
matching bit strings and never on integer types; comparisons always produce
`BOOL`. -/
inductive BinaryOp where
  /-- Addition. -/
  | add
  /-- Subtraction. -/
  | sub
  /-- Multiplication. -/
  | mul
  /-- Truncating division. -/
  | div
  /-- Truncating remainder. -/
  | mod
  /-- Integer exponentiation. -/
  | pow
  /-- Bitwise `AND`. -/
  | and
  /-- Bitwise `OR`. -/
  | or
  /-- Bitwise `XOR`. -/
  | xor
  /-- Equality. -/
  | eq
  /-- Inequality. -/
  | ne
  /-- Less than. -/
  | lt
  /-- Less than or equal. -/
  | le
  /-- Greater than. -/
  | gt
  /-- Greater than or equal. -/
  | ge
deriving Repr, DecidableEq

namespace BinaryOp

/-- Whether the operator is a comparison; comparisons produce `BOOL`. -/
def isComparison : BinaryOp → Bool
  | .eq | .ne | .lt | .le | .gt | .ge => true
  | _ => false

/-- Whether the operator is arithmetic on integers. -/
def isArithmetic : BinaryOp → Bool
  | .add | .sub | .mul | .div | .mod | .pow => true
  | _ => false

/-- Whether the operator is bitwise on `BOOL` or matching bit strings. -/
def isBitwise : BinaryOp → Bool
  | .and | .or | .xor => true
  | _ => false

end BinaryOp

/-- The provenance of an explicit cast. The distinction is dumped for
traceability; typing treats both kinds identically. -/
inductive CastKind where
  /-- Inserted by lowering for an implicit source conversion. -/
  | implicit
  /-- Written explicitly in the source. -/
  | explicit
deriving Repr, DecidableEq

/-- The exact standard functions included in the core. Arithmetic, bitwise, and
comparison standard functions are represented by the operator nodes; other
standard functions are excluded until Phase 6. -/
inductive StdFn where
  /-- `ABS`. -/
  | abs
  /-- `SEL`. -/
  | sel
  /-- `MIN`. -/
  | min
  /-- `MAX`. -/
  | max
  /-- `LIMIT`. -/
  | limit
  /-- `MUX`. -/
  | mux
deriving Repr, DecidableEq

/-- A resolved callee. Calls are always direct: a function index or a standard
function. Indirect calls, methods, and function blocks are outside the core. -/
inductive Callee where
  /-- A user function, by index into the unit's function list. -/
  | function (index : Nat) (span : Span)
  /-- An exact standard function. -/
  | std (fn : StdFn) (span : Span)
deriving Repr, DecidableEq

/-- A lowered HIR expression. -/
inductive Expr where
  /-- A typed literal. -/
  | literal (lit : Literal) (span : Span)
  /-- A variable read. -/
  | var (id : VarId) (span : Span)
  /-- A unary operation. -/
  | unary (op : UnaryOp) (operand : Expr) (span : Span)
  /-- A binary operation. -/
  | binary (op : BinaryOp) (lhs rhs : Expr) (span : Span)
  /-- A direct call. Arguments are in formal-parameter order. -/
  | call (callee : Callee) (args : List Expr) (span : Span)
  /-- An explicit conversion (implicit or source-written). -/
  | cast (operand : Expr) (to : Ty) (kind : CastKind) (span : Span)

namespace Expr

/-- The source span of an expression. -/
def span : Expr → Span
  | .literal _ s => s
  | .var _ s => s
  | .unary _ _ s => s
  | .binary _ _ _ s => s
  | .call _ _ s => s
  | .cast _ _ _ s => s

/-- The constant integer value of a literal, unary sign, or nested sign. This
is the lowered form of a `CASE` label or `FOR` step that the front end folded.
-/
def constInt? : Expr → Option Int
  | .literal lit _ => if lit.ty.isInt then some lit.value else none
  | .unary .neg e _ => (constInt? e).map (fun v => -v)
  | .unary .pos e _ => constInt? e
  | _ => none

end Expr

/-- An assignable HIR place. The core has scalar variable places only. -/
structure Place where
  /-- The resolved variable. -/
  id : VarId
  /-- The source span. -/
  span : Span
deriving Repr, DecidableEq

/-- A folded `CASE` label. `low` and the optional `high` are constant integers
in the selector's canonical type; the checker validates range, order, and
non-overlap. -/
structure CaseLabel where
  /-- The low bound, or the only label value. -/
  low : Int
  /-- The optional high bound of a range label. -/
  high : Option Int
  /-- The source span. -/
  span : Span
deriving Repr, DecidableEq

namespace CaseLabel

/-- The inclusive `[low, high]` bounds, with a single label treated as a
one-point range. -/
def bounds (label : CaseLabel) : Int × Int :=
  (label.low, label.high.getD label.low)

/-- Whether two label ranges overlap. For ordered labels this is reflexive. -/
def overlap (first second : CaseLabel) : Bool :=
  let (firstLow, firstHigh) := first.bounds
  let (secondLow, secondHigh) := second.bounds
  !(firstHigh < secondLow || secondHigh < firstLow)

/-- Every label bound is representable in the selector type. -/
def inRangeB (label : CaseLabel) (ty : Ty) : Bool :=
  ty.holdsB label.low && label.high.all (fun high => ty.holdsB high)

/-- A range label is ordered. A single label is trivially ordered. -/
def orderedB (label : CaseLabel) : Bool :=
  label.high.all (fun high => decide (label.low ≤ high))

/-- The number of occurrences of a label in a list. -/
def countIn (label : CaseLabel) : List CaseLabel → Nat
  | [] => 0
  | other :: rest => (if other = label then 1 else 0) + countIn label rest

/-- Overlap is symmetric. -/
theorem overlap_comm (first second : CaseLabel) :
    first.overlap second = second.overlap first := by
  unfold CaseLabel.overlap CaseLabel.bounds
  cases first.high <;> cases second.high <;> simp [Bool.or_comm]

/-- An ordered label overlaps itself. -/
theorem overlap_self {label : CaseLabel}
    (h : ∀ high, label.high = some high → label.low ≤ high) :
    label.overlap label = true := by
  unfold CaseLabel.overlap CaseLabel.bounds
  cases high : label.high with
  | none =>
      simp
  | some value =>
      have hle := h value high
      have hlt : ¬ value < label.low := by omega
      simp [hlt]

end CaseLabel

mutual
  /-- A lowered HIR statement. -/
  inductive Stmt where
    /-- Assignment to a scalar place. -/
    | assign (place : Place) (value : Expr) (span : Span)
    /-- A call used as a statement; the result, if any, is discarded. -/
    | call (call : Expr) (span : Span)
    /-- Return from the current function or cycle body. -/
    | return_ (span : Span)
    /-- Exit the innermost loop. -/
    | exit (span : Span)
    /-- Continue the innermost loop. -/
    | continue_ (span : Span)
    /-- The empty statement. -/
    | empty (span : Span)
    /-- `IF`/`ELSIF`/`ELSE`; there is always at least one arm. -/
    | ifThenElse (arms : List IfArm) (elseBody : List Stmt) (span : Span)
    /-- `CASE` over folded integer labels. -/
    | case (selector : Expr) (arms : List CaseArm) (elseBody : List Stmt) (span : Span)
    /-- `FOR` with an optional `BY` step (default `1`). -/
    | forLoop (var : Place) (start stop : Expr) (step : Option Expr)
        (body : List Stmt) (span : Span)
    /-- `WHILE`. -/
    | whileLoop (cond : Expr) (body : List Stmt) (span : Span)
    /-- `REPEAT`/`UNTIL`. -/
    | repeatLoop (body : List Stmt) (cond : Expr) (span : Span)

  /-- One `IF` or `ELSIF` arm. -/
  inductive IfArm where
    | mk (cond : Expr) (body : List Stmt) (span : Span)

  /-- One `CASE` selection: labels and a body. -/
  inductive CaseArm where
    | mk (labels : List CaseLabel) (body : List Stmt) (span : Span)
end

namespace IfArm

/-- The arm condition. -/
def cond : IfArm → Expr
  | .mk c _ _ => c

/-- The arm body. -/
def body : IfArm → List Stmt
  | .mk _ b _ => b

/-- The arm span. -/
def span : IfArm → Span
  | .mk _ _ s => s

end IfArm

namespace CaseArm

/-- The selection labels. -/
def labels : CaseArm → List CaseLabel
  | .mk ls _ _ => ls

/-- The selection body. -/
def body : CaseArm → List Stmt
  | .mk _ b _ => b

/-- The selection span. -/
def span : CaseArm → Span
  | .mk _ _ s => s

end CaseArm

/-- The statement span. -/
def Stmt.span : Stmt → Span
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

/-- A function parameter. -/
structure Param where
  /-- The canonical source name (uppercase), retained for diagnostics and the
  dump. Typing uses the parameter's position. -/
  name : String
  /-- The parameter type. -/
  ty : Ty
  /-- The declaration span. -/
  span : Span

/-- A local variable declaration. -/
structure Local where
  /-- The canonical source name. -/
  name : String
  /-- The declared type. -/
  ty : Ty
  /-- The optional initializer. -/
  init : Option Expr
  /-- The declaration span. -/
  span : Span

/-- A global root declaration. -/
structure Global where
  /-- The canonical source name. -/
  name : String
  /-- The declared type. -/
  ty : Ty
  /-- The optional initializer. -/
  init : Option Expr
  /-- The declaration span. -/
  span : Span

/-- A core function declaration. -/
structure Function where
  /-- The canonical source name. -/
  name : String
  /-- The optional result type. -/
  resultType : Option Ty
  /-- `VAR_INPUT` parameters. -/
  params : List Param
  /-- `VAR` locals. -/
  locals : List Local
  /-- The body. -/
  body : List Stmt
  /-- The declaration span. -/
  span : Span

/-- A core program declaration: scalar fields and one cycle body. -/
structure Program where
  /-- The canonical source name. -/
  name : String
  /-- Scalar program fields. -/
  locals : List Local
  /-- The cycle body. -/
  body : List Stmt
  /-- The declaration span. -/
  span : Span

/-- A core unit: global roots, functions, and exactly one program. -/
structure HirUnit where
  /-- Global roots. -/
  globals : List Global
  /-- Functions, in dump order. -/
  functions : List Function
  /-- The single program. -/
  program : Program
  /-- The unit span. -/
  span : Span

end Stc.Hir
