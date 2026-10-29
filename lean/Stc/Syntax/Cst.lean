/-
The lossless CST contract for the core surface language (P0.1).

The contract states that token and trivia spans cover the source exactly, in
order, with no gaps or overlaps, and that every node span is the extent of its
leaves. The full rowan representation remains a contract-level abstraction: the
grammar relation in `Stc.Grammar.Derives` consumes the trivia-free projection
`Cst.stripTrivia`, while the lossless coverage predicate is stated over the
full tree.
-/
import Stc.Syntax.Tokens

namespace Stc.Syntax

/-- The production tags used by the core CST. The names mirror the core EBNF
productions in [Grammar](../../../specification/Syntax/Grammar.md). -/
inductive NodeKind where
  | sourceFile
  | topItem
  | varSection
  | varDecl
  | functionDecl
  | programDecl
  | typeSpec
  | statementList
  | assignmentStatement
  | callStatement
  | returnStatement
  | exitStatement
  | continueStatement
  | ifStatement
  | elsifClause
  | elseClause
  | caseStatement
  | caseSelection
  | caseList
  | caseElement
  | forStatement
  | whileStatement
  | repeatStatement
  | expression
  | orExpression
  | xorExpression
  | andExpression
  | compareExpression
  | equExpression
  | addExpression
  | term
  | powerExpression
  | unaryExpression
  | postfixExpression
  | primaryExpression
  | parenExpression
  | literalExpression
  | constantExpression
  | path
  | variable
  | place
  | variableOrCall
  | callExpression
  | argumentList
  | argument
deriving Repr, DecidableEq

/-- A concrete syntax tree: tokens and trivia are leaves, nodes carry a
production tag, a span, and ordered children. -/
inductive Cst where
  | token (value : Token)
  | trivia (value : Trivia)
  | node (kind : NodeKind) (span : Span) (children : List Cst)
deriving Repr

namespace Cst

/-- The source span of a CST item. -/
def span : Cst → Span
  | .token t => t.span
  | .trivia t => t.span
  | .node _ s _ => s

/-- Whether the item is trivia. -/
def isTrivia : Cst → Bool
  | .trivia _ => true
  | _ => false

/-- The direct children of a node; leaves have none. -/
def children : Cst → List Cst
  | .node _ _ cs => cs
  | _ => []

/-- The structural size of a CST item. It is the termination measure for the
span lemmas over derivations. -/
def size : Cst → Nat
  | .token _ => 1
  | .trivia _ => 1
  | .node _ _ cs => 1 + (cs.map size).sum

private theorem le_sum_of_mem {a : Nat} {l : List Nat} (h : a ∈ l) : a ≤ l.sum := by
  induction l with
  | nil => simp at h
  | cons b bs ih =>
      simp only [List.mem_cons] at h
      simp only [List.sum_cons]
      rcases h with rfl | h
      · omega
      · have := ih h
        omega

/-- A direct child is strictly smaller than its parent. -/
theorem size_child_lt {c child : Cst} (hm : child ∈ c.children) : child.size < c.size := by
  cases c with
  | token _ => simp [children] at hm
  | trivia _ => simp [children] at hm
  | node k s cs =>
      simp [children] at hm
      simp only [size]
      have hmem : child.size ∈ cs.map size := List.mem_map_of_mem hm
      have hle : child.size ≤ (cs.map size).sum := le_sum_of_mem hmem
      omega

/-- Remove trivia leaves, recursively. `none` means the item itself is trivia;
a node is retained even when all of its children are trivia. -/
def stripTrivia : Cst → Option Cst
  | .trivia _ => none
  | .token t => some (.token t)
  | .node k s cs => some (.node k s (cs.filterMap stripTrivia))

/-- The significant children of a node (trivia removed). This is the child list
that `Stc.Grammar.Derives` matches. -/
def significantChildren (cs : List Cst) : List Cst :=
  cs.filterMap stripTrivia

/-- All token and trivia spans under an item, in source order. -/
def leaves : Cst → List Span
  | .token t => [t.span]
  | .trivia t => [t.span]
  | .node _ _ cs => (cs.map leaves).flatten

/-- All node spans under an item, in pre-order. -/
def nodeSpans : Cst → List Span
  | .node _ s cs => s :: (cs.map nodeSpans).flatten
  | _ => []

/-- Removing trivia preserves the item's span. -/
theorem stripTrivia_span {c c' : Cst} (h : stripTrivia c = some c') :
    c'.span = c.span := by
  cases c with
  | trivia t => simp [stripTrivia] at h
  | token t =>
      simp only [stripTrivia, Option.some.injEq] at h
      subst c'
      rfl
  | node k s cs =>
      simp only [stripTrivia, Option.some.injEq] at h
      subst c'
      rfl

/-- A significant child of a node is the trivia-free projection of one of its
direct children. -/
theorem mem_significantChildren {child : Cst} {cs : List Cst}
    (h : child ∈ significantChildren cs) :
    ∃ parent ∈ cs, stripTrivia parent = some child := by
  unfold significantChildren at h
  exact List.mem_filterMap.mp h

end Cst

/-- Contiguous, ordered coverage of `spans` from `pos` to `stop`. -/
inductive CoversFrom (file : FileId) : List Span → Nat → Nat → Prop where
  | nil {pos stop : Nat} : pos = stop → CoversFrom file [] pos stop
  | cons {s : Span} {rest : List Span} {pos stop : Nat} :
      s.file = file →
      s.start = pos →
      s.WellFormed →
      CoversFrom file rest s.stop stop →
      CoversFrom file (s :: rest) pos stop

namespace CoversFrom

/-- A covering sequence does not extend past its end bound. -/
theorem pos_le_stop {file : FileId} {spans : List Span} {pos stop : Nat}
    (h : CoversFrom file spans pos stop) : pos ≤ stop := by
  induction h with
  | nil h => omega
  | cons _ hstart hvalid hrest ih =>
      unfold Span.WellFormed at hvalid
      omega

/-- Every span in a covering sequence lies inside the bounds. -/
theorem mem_bounds {file : FileId} {spans : List Span} {pos stop : Nat}
    (h : CoversFrom file spans pos stop) :
    ∀ {s : Span}, s ∈ spans →
      s.file = file ∧ pos ≤ s.start ∧ s.stop ≤ stop ∧ s.WellFormed := by
  induction h with
  | nil => intro s hm; simp at hm
  | cons hfile hstart hvalid hrest ih =>
      intro s hm
      simp only [List.mem_cons] at hm
      rcases hm with rfl | hm
      · exact ⟨hfile, by omega, hrest.pos_le_stop, hvalid⟩
      · have ih' := ih hm
        unfold Span.WellFormed at hvalid
        exact ⟨ih'.1, by omega, ih'.2.2.1, ih'.2.2.2⟩

end CoversFrom

/-- The lossless contract: the leaves of `c` cover `[0, sourceLength)` exactly,
in order, with no gaps or overlaps. -/
def Lossless (c : Cst) (file : FileId) (sourceLength : Nat) : Prop :=
  CoversFrom file c.leaves 0 sourceLength

namespace Lossless

/-- Every leaf of a lossless CST lies within the source. -/
theorem leaf_bounds {c : Cst} {file : FileId} {sourceLength : Nat}
    (h : Lossless c file sourceLength) {s : Span} (hm : s ∈ c.leaves) :
    s.file = file ∧ s.stop ≤ sourceLength ∧ s.WellFormed := by
  have hb := CoversFrom.mem_bounds h hm
  exact ⟨hb.1, hb.2.2.1, hb.2.2.2⟩

end Lossless

namespace Cst

/-- Node spans agree with the tree: each node span is well formed and contains
the spans of its direct children. Together with `Lossless`, this gives the
source-span mapping used by the grammar relation. -/
inductive WellFormedNodes : Cst → Prop where
  | token {t : Token} : t.span.WellFormed → WellFormedNodes (.token t)
  | trivia {t : Trivia} : t.span.WellFormed → WellFormedNodes (.trivia t)
  | node {kind : NodeKind} {s : Span} {cs : List Cst} :
      s.WellFormed →
      (∀ c ∈ cs, s.contains c.span) →
      (∀ c ∈ cs, WellFormedNodes c) →
      WellFormedNodes (.node kind s cs)

namespace WellFormedNodes

/-- The span of a well-formed tree is well formed. -/
theorem span_wellFormed {c : Cst} (h : c.WellFormedNodes) : c.span.WellFormed := by
  cases h with
  | token h => exact h
  | trivia h => exact h
  | node h _ _ => exact h

/-- A node span contains each direct child's span. -/
theorem contains_child {c child : Cst} (h : c.WellFormedNodes)
    (hm : child ∈ c.children) : c.span.contains child.span := by
  cases h with
  | token _ => simp [Cst.children] at hm
  | trivia _ => simp [Cst.children] at hm
  | node _ hcontain _ => exact hcontain child hm

/-- Every direct child of a well-formed tree is well formed. -/
theorem child {c child : Cst} (h : c.WellFormedNodes) (hm : child ∈ c.children) :
    child.WellFormedNodes := by
  cases h with
  | token _ => simp [Cst.children] at hm
  | trivia _ => simp [Cst.children] at hm
  | node _ _ hnested => exact hnested child hm

/-- A node span contains each significant child's span. -/
theorem contains_significantChild {c child : Cst} (h : c.WellFormedNodes)
    (hm : child ∈ significantChildren c.children) :
    c.span.contains child.span := by
  rcases mem_significantChildren hm with ⟨parent, hparent, hstrip⟩
  have hcont := h.contains_child hparent
  have hspan : child.span = parent.span := stripTrivia_span hstrip
  rw [hspan]
  exact hcont

end WellFormedNodes

/-- Every node span lies within the source file bounds. -/
def NodeSpansWithin (c : Cst) (file : FileId) (sourceLength : Nat) : Prop :=
  ∀ s ∈ c.nodeSpans,
    s.file = file ∧ s.start ≤ sourceLength ∧ s.stop ≤ sourceLength ∧ s.WellFormed

/-- The full CST contract: lossless leaf coverage, node-span coherence, and
node spans bounded by the source. -/
def WellFormed (c : Cst) (file : FileId) (sourceLength : Nat) : Prop :=
  Lossless c file sourceLength ∧ c.WellFormedNodes ∧ c.NodeSpansWithin file sourceLength

namespace WellFormed

/-- A well-formed CST is lossless. -/
theorem lossless {c : Cst} {file : FileId} {sourceLength : Nat}
    (h : c.WellFormed file sourceLength) : Lossless c file sourceLength :=
  h.1

/-- A well-formed CST has coherent node spans. -/
theorem nodes {c : Cst} {file : FileId} {sourceLength : Nat}
    (h : c.WellFormed file sourceLength) : c.WellFormedNodes :=
  h.2.1

/-- A well-formed CST's node spans lie within the source. -/
theorem node_bounds {c : Cst} {file : FileId} {sourceLength : Nat}
    (h : c.WellFormed file sourceLength) {s : Span} (hm : s ∈ c.nodeSpans) :
    s.file = file ∧ s.start ≤ sourceLength ∧ s.stop ≤ sourceLength ∧ s.WellFormed :=
  h.2.2 s hm

/-- A well-formed CST's own span lies within the source. -/
theorem span_bounds {c : Cst} {file : FileId} {sourceLength : Nat}
    (h : c.WellFormed file sourceLength) :
    c.span.file = file ∧ c.span.start ≤ sourceLength ∧ c.span.stop ≤ sourceLength ∧
      c.span.WellFormed := by
  cases c with
  | token t =>
      have hb := Lossless.leaf_bounds h.1 (s := t.span) (by simp [Cst.leaves])
      have hw : t.span.start ≤ t.span.stop := hb.2.2
      exact ⟨hb.1, by
        change t.span.start ≤ sourceLength
        omega, hb.2.1, hb.2.2⟩
  | trivia t =>
      have hb := Lossless.leaf_bounds h.1 (s := t.span) (by simp [Cst.leaves])
      have hw : t.span.start ≤ t.span.stop := hb.2.2
      exact ⟨hb.1, by
        change t.span.start ≤ sourceLength
        omega, hb.2.1, hb.2.2⟩
  | node k s cs =>
      exact h.node_bounds (s := s) (by simp [Cst.nodeSpans])

end WellFormed

end Cst

end Stc.Syntax
