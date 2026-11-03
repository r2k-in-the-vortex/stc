/-
Trust audit for the filtered Aeneas exact model.

This module is compiled only in the non-default `lean/aeneas` package. It
inventories the four generated external axioms and the refinement propositions
stated in `Bridge/Exact/Refinement.lean`. All 24 propositions now have completed
proofs in `Bridge/Exact/Soundness.lean`; the four external axioms remain TCB
assumptions recorded here with their exact-side models.

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

/-- The four external axioms emitted by Aeneas for the filtered model. Each is
recorded in the TCB with the exact-side model it is assumed to satisfy; the
equation cannot be proved inside Lean because the generated function is
opaque. -/
def externalAxioms : List AxiomEntry :=
  [ { axiomName := "I64::from(u8)"
      statement := fromU8Refines
      status := "TCB: modelled by Stc.Bridge.Exact.fromU8 (natural embedding); opaque generated function" },
    { axiomName := "I64::rem_euclid"
      statement := remEuclidRefines
      status := "TCB: modelled by Stc.Bridge.Exact.remEuclid (Euclidean remainder) on the nonzero, non-MIN/-1 domain" },
    { axiomName := "I128::checked_neg"
      statement := checkedNegRefines
      status := "TCB: modelled by Stc.Bridge.Exact.checkedNeg (fails exactly at i128::MIN)" },
    { axiomName := "I128::abs"
      statement := i128AbsRefines
      status := "TCB: modelled by Stc.Bridge.Exact.iabs on the guarded non-i128::MIN domain" } ]

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
      status := "proved: modulus_refines" },
    { obligationName := "minimumRefines", statement := minimumRefines
      status := "proved: minimum_refines" },
    { obligationName := "maximumRefines", statement := maximumRefines
      status := "proved: maximum_refines" },
    { obligationName := "fitsRefines", statement := fitsRefines
      status := "proved: fits_refines" },
    { obligationName := "maskRefines", statement := maskRefines
      status := "proved: mask_refines" },
    { obligationName := "countRefines", statement := countRefines
      status := "proved: count_refines" },
    { obligationName := "checkedRefines", statement := checkedRefines
      status := "proved: checked_refines" } ]

/-- Checked arithmetic operations. -/
def arithmeticObligations : List ObligationEntry :=
  [ { obligationName := "addRefines", statement := addRefines
      status := "proved: add_refines" },
    { obligationName := "subRefines", statement := subRefines
      status := "proved: sub_refines" },
    { obligationName := "mulRefines", statement := mulRefines
      status := "proved: mul_refines" },
    { obligationName := "negRefines", statement := negRefines
      status := "proved: neg_refines" },
    { obligationName := "divRefines", statement := divRefines
      status := "proved: div_refines" },
    { obligationName := "moduloRefines", statement := moduloRefines
      status := "proved: modulo_refines" },
    { obligationName := "absRefines", statement := absRefines
      status := "proved: abs_refines" } ]

/-- Bitwise, shift, and rotate operations. -/
def bitObligations : List ObligationEntry :=
  [ { obligationName := "bitAndRefines", statement := bitAndRefines
      status := "proved: bitAnd_refines" },
    { obligationName := "bitOrRefines", statement := bitOrRefines
      status := "proved: bitOr_refines" },
    { obligationName := "bitXorRefines", statement := bitXorRefines
      status := "proved: bitXor_refines" },
    { obligationName := "bitNotRefines", statement := bitNotRefines
      status := "proved: bitNot_refines" },
    { obligationName := "shlRefines", statement := shlRefines
      status := "proved: shl_refines" },
    { obligationName := "shrRefines", statement := shrRefines
      status := "proved: shr_refines" },
    { obligationName := "rolRefines", statement := rolRefines
      status := "proved: rol_refines" },
    { obligationName := "rorRefines", statement := rorRefines
      status := "proved: ror_refines" } ]

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
def provedObligations : List ObligationEntry := allObligations

/-- The obligations that remain open in this batch. -/
def openObligations : List ObligationEntry := []

/-- Number of stated refinement obligations (proved and open). -/
def obligationCount : Nat := allObligations.length

/-- Number of obligations with completed proofs. -/
def provedCount : Nat := provedObligations.length

/-- Number of open obligations. -/
def openCount : Nat := openObligations.length

/-- The count identity checked by the audit. -/
example : provedCount + openCount = obligationCount := by
  simp [provedCount, openCount, obligationCount, allObligations, provedObligations,
    openObligations]

end Stc.Bridge.Exact
