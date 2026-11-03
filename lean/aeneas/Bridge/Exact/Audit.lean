/-
Trust audit for the filtered Aeneas exact model.

This module is compiled only in the non-default `lean/aeneas` package. It
inventories the four generated external axioms and the refinement propositions
stated in `Bridge/Exact/Refinement.lean`, separating the two propositions with
completed proofs from the open obligations.

The generated axioms are still axioms: listing them here neither discharges
them nor turns any record into a V1 claim. A V1 claim requires completed
refinement proofs, axiom discharge or an accepted TCB record, and owner review.
-/

import Bridge.Exact.Refinement

namespace Stc.Bridge.Exact

/-- One external operation left opaque by the generated model. -/
structure AxiomEntry where
  /-- The generated axiom name. -/
  axiomName : String
  /-- The exact-side equation that the axiom would have to satisfy. -/
  statement : Prop
  /-- Current evidence status; never "proved" without a completed proof. -/
  status : String

/-- The four external axioms emitted by Aeneas for the filtered model. -/
def externalAxioms : List AxiomEntry :=
  [ { axiomName := "I64::from(u8)"
      statement := fromU8Refines
      status := "open: generated axiom; equation stated, not proved" },
    { axiomName := "I64::rem_euclid"
      statement := remEuclidRefines
      status := "open: generated axiom; equation stated on the nonzero, non-MIN/-1 domain, not proved" },
    { axiomName := "I128::checked_neg"
      statement := checkedNegRefines
      status := "open: generated axiom; equation stated, not proved" },
    { axiomName := "I128::abs"
      statement := i128AbsRefines
      status := "open: generated axiom; equation stated on the guarded domain, not proved" } ]

/-- One refinement proposition for the generated surface. -/
structure ObligationEntry where
  /-- The proposition name. -/
  obligationName : String
  /-- The proposition itself. -/
  statement : Prop
  /-- Current evidence status. -/
  status : String

/-- Generated helper operations. -/
def helperObligations : List ObligationEntry :=
  [ { obligationName := "modulusRefines", statement := modulusRefines
      status := "open: stated, not proved" },
    { obligationName := "minimumRefines", statement := minimumRefines
      status := "open: stated, not proved" },
    { obligationName := "maximumRefines", statement := maximumRefines
      status := "open: stated, not proved" },
    { obligationName := "fitsRefines", statement := fitsRefines
      status := "open: stated, not proved" },
    { obligationName := "maskRefines", statement := maskRefines
      status := "open: stated, not proved" },
    { obligationName := "countRefines", statement := countRefines
      status := "open: stated, not proved" },
    { obligationName := "checkedRefines", statement := checkedRefines
      status := "open: stated, not proved" } ]

/-- Checked arithmetic operations. -/
def arithmeticObligations : List ObligationEntry :=
  [ { obligationName := "addRefines", statement := addRefines
      status := "open: stated, not proved" },
    { obligationName := "subRefines", statement := subRefines
      status := "open: stated, not proved" },
    { obligationName := "mulRefines", statement := mulRefines
      status := "open: stated, not proved" },
    { obligationName := "negRefines", statement := negRefines
      status := "open: stated, not proved" },
    { obligationName := "divRefines", statement := divRefines
      status := "open: stated, not proved" },
    { obligationName := "moduloRefines", statement := moduloRefines
      status := "open: stated, not proved" },
    { obligationName := "absRefines", statement := absRefines
      status := "open: stated, not proved" } ]

/-- Bitwise, shift, and rotate operations. -/
def bitObligations : List ObligationEntry :=
  [ { obligationName := "bitAndRefines", statement := bitAndRefines
      status := "open: stated, not proved" },
    { obligationName := "bitOrRefines", statement := bitOrRefines
      status := "open: stated, not proved" },
    { obligationName := "bitXorRefines", statement := bitXorRefines
      status := "open: stated, not proved" },
    { obligationName := "bitNotRefines", statement := bitNotRefines
      status := "open: stated, not proved" },
    { obligationName := "shlRefines", statement := shlRefines
      status := "open: stated, not proved" },
    { obligationName := "shrRefines", statement := shrRefines
      status := "open: stated, not proved" },
    { obligationName := "rolRefines", statement := rolRefines
      status := "open: stated, not proved" },
    { obligationName := "rorRefines", statement := rorRefines
      status := "open: stated, not proved" } ]

/-- Comparison and selection. -/
def selectionObligations : List ObligationEntry :=
  [ { obligationName := "compareRefines", statement := compareRefines
      status := "proved: compare_refines" },
    { obligationName := "selRefines", statement := selRefines
      status := "proved: sel_refines" } ]

/-- Every stated obligation, with its status. -/
def allObligations : List ObligationEntry :=
  helperObligations ++ arithmeticObligations ++ bitObligations ++
    selectionObligations

/-- The obligations with completed proofs. -/
def provedObligations : List ObligationEntry :=
  [ { obligationName := "compareRefines", statement := compareRefines
      status := "proved: compare_refines" },
    { obligationName := "selRefines", statement := selRefines
      status := "proved: sel_refines" } ]

/-- The obligations that remain open in this batch. -/
def openObligations : List ObligationEntry :=
  helperObligations ++ arithmeticObligations ++ bitObligations

/-- Number of stated refinement obligations (proved and open). -/
def obligationCount : Nat := allObligations.length

/-- Number of obligations with completed proofs. -/
def provedCount : Nat := provedObligations.length

/-- Number of open obligations. -/
def openCount : Nat := openObligations.length

/-- The count identity checked by the audit. -/
example : provedCount + openCount = obligationCount := by
  simp [provedCount, openCount, obligationCount, allObligations, provedObligations,
    openObligations, helperObligations, arithmeticObligations, bitObligations,
    selectionObligations]

end Stc.Bridge.Exact
