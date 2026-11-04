/-
Trust audit for the filtered Aeneas exact model.

This module is compiled only in the non-default `lean/aeneas` package. It
inventories the twelve axiom declarations of the generated model and the
bridge's five string TCB equations (plus the opaque `strScalars` decoding
function), and the refinement propositions stated in
`Bridge/Exact/Refinement.lean` and `Bridge/Exact/WideRefinement.lean`. All 32
propositions have completed proofs; the external axioms remain TCB assumptions
recorded here with their exact-side models. `Bridge/Exact/Kernel.lean` composes
the 32 proofs into the bundled `exact_kernel_refines` statement.

The generated axioms are still axioms: listing them here neither discharges
them nor turns any record into a V1 claim. A V1 claim requires completed
refinement proofs, axiom discharge or an accepted TCB record, and owner review.
-/

import Bridge.Exact.Kernel

namespace Stc.Bridge.Exact

open Aeneas Aeneas.Std

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

/-- The five string TCB axioms added by the widened generated model. Each
states what an opaque standard-library operation computes over the shared
string model; none is discharged. -/
def wideTcbAxioms : List AxiomEntry :=
  [ { axiomName := "strScalars_utf8ByteLength"
      statement := ∀ text : Str,
        Stc.Exact.utf8ByteLength (strScalars text) = text.val.length
      status := "TCB: the decoded scalar sequence encodes to the stored byte length (UTF-8 validity invariant)" },
    { axiomName := "str::len"
      statement := ∀ text : Str,
        ResultRefinesUsize (exact.core.str.Str.len text) text.val.length
      status := "TCB: `str::len` returns the stored byte length" },
    { axiomName := "str::is_ascii"
      statement := ∀ text : Str,
        ResultRefinesBool (exact.core.str.Str.is_ascii text) (isAsciiBytes text.val)
      status := "TCB: `str::is_ascii` tests every stored byte against 0x80" },
    { axiomName := "str::chars::count"
      statement := ∀ text : Str,
        ResultRefinesUsize
          (do let chars ← core.str.Str.chars text
              exact.core.str.iter.Chars.Insts.CoreIterTraitsIteratorIteratorChar.count chars)
          (strScalars text).length
      status := "TCB: `str::chars` followed by `Iterator::count` counts the decoded scalars" },
    { axiomName := "str::encode_utf16::count"
      statement := ∀ text : Str,
        ResultRefinesUsize
          (do let units ← exact.core.str.Str.encode_utf16 text
              exact.core.str.iter.EncodeUtf16.Insts.CoreIterTraitsIteratorIteratorU16.count units)
          (Stc.Exact.utf16Length (strScalars text))
      status := "TCB: `str::encode_utf16` followed by `Iterator::count` counts UTF-16 code units" } ]

/-- All TCB axiom equations recorded for the generated model. -/
def allAxioms : List AxiomEntry := externalAxioms ++ wideTcbAxioms

/-- A TCB declaration that states no equation: an opaque model function or a
generated declaration that no proved obligation reaches. -/
structure TcbDeclaration where
  /-- The declaration name as it appears in the generated model or bridge. -/
  declarationName : String
  /-- Why it is in the TCB and whether the obligation closure uses it. -/
  status : String

/-- The generated model contains twelve `axiom` declarations. Nine are
operation axioms: the four external scalar operations inventoried in
`externalAxioms`, and the five string operations whose equations (and the
supporting UTF-8 byte-length invariant) are recorded in `wideTcbAxioms`. The
remaining three generated declarations state no operation equation and are not
reachable from any proved obligation. `strScalars`, the shared decoding
function used to state the string equations, is itself declared as an axiom by
`Bridge.Exact.StrModel`; it has no defining equation to record. -/
def nonEquationTcbDeclarations : List TcbDeclaration :=
  [ { declarationName := "strScalars"
      status := "TCB: opaque decoding function `Str → List Nat`; declared as an axiom, no defining equation" },
    { declarationName := "Shared0T.Insts.CoreFmtDisplay.fmt"
      status := "generated axiom; outside the proved obligation closure (no Display formatting is refined)" },
    { declarationName := "Str.Insts.CoreFmtDisplay.fmt"
      status := "generated axiom; outside the proved obligation closure (no Display formatting is refined)" },
    { declarationName := "core.str.iter.EncodeUtf16"
      status := "generated opaque type declaration; the encode_utf16 axiom observes it only through the count equation" } ]

/-- Number of `axiom` declarations in `StcGenerated.Exact.Model`. -/
def generatedAxiomCount : Nat := 12

/-- Number of equation entries in `allAxioms`. -/
def tcbEquationCount : Nat := allAxioms.length

/-- Every generated axiom declaration is accounted for: nine operation
axioms (four recorded individually, five covered by the string equations) and
three non-equation declarations. -/
example : tcbEquationCount = 9 ∧ generatedAxiomCount = 12 ∧
    nonEquationTcbDeclarations.length = 4 := by
  decide

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

/-- Operations added by the widened generated model. -/
def wideObligations : List ObligationEntry :=
  [ { obligationName := "convertRefines", statement := convertRefines
      status := "proved: convert_refines" },
    { obligationName := "minListRefines", statement := minListRefines
      status := "proved: min_list_refines" },
    { obligationName := "maxListRefines", statement := maxListRefines
      status := "proved: max_list_refines" },
    { obligationName := "limitRefines", statement := limitRefines
      status := "proved: limit_refines" },
    { obligationName := "muxRefines", statement := muxRefines
      status := "proved: mux_refines" },
    { obligationName := "lenRefines", statement := lenRefines
      status := "proved: len_refines" },
    { obligationName := "lenMaxRefines", statement := lenMaxRefines
      status := "proved: len_max_refines" },
    { obligationName := "lenCodeUnitRefines", statement := lenCodeUnitRefines
      status := "proved: len_code_unit_refines" } ]

/-- Every stated obligation, with its status. -/
def allObligations : List ObligationEntry :=
  helperObligations ++ arithmeticObligations ++ bitObligations ++
    selectionObligations ++ wideObligations

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

/-- The bundled statement about the generated kernel, proved in
`Bridge/Exact/Kernel.lean`. -/
def kernelStatement : Prop := exactKernelRefines

/-- The bundle has a completed proof. -/
theorem kernelStatement_proved : kernelStatement := exact_kernel_refines

end Stc.Bridge.Exact
