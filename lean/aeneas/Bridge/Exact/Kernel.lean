/-
Bundled kernel-refinement statement for the generated exact model.

This module is compiled only in the non-default `lean/aeneas` package. It
composes the per-operation refinement propositions into one statement about the
generated functions: `StcGenerated.Exact.Model` is extracted from
`crates/stc-codegen/src/exact.rs`, the single scalar kernel that the analyzer's
constant folder (`consteval.rs`) and code generation (`cast_value`/
`int_convert`) now share. The bundled theorem therefore covers every scalar
operation that actually runs in the build, without changing any per-operation
proof or adding a new assumption.

No proof holes are used.
-/

import Bridge.Exact.WideSoundness

namespace Stc.Bridge.Exact

/-- Every generated exact operation refines its `Stc.Exact` model. The
conjunction orders the obligations as the audit lists them: generated helpers,
checked arithmetic, bitwise/shift/rotate, comparison/selection, and the wide
operations added by the regenerated model. -/
def exactKernelRefines : Prop :=
  modulusRefines ∧ minimumRefines ∧ maximumRefines ∧ fitsRefines ∧
  maskRefines ∧ countRefines ∧ checkedRefines ∧
  addRefines ∧ subRefines ∧ mulRefines ∧ negRefines ∧ divRefines ∧
  moduloRefines ∧ absRefines ∧
  bitAndRefines ∧ bitOrRefines ∧ bitXorRefines ∧ bitNotRefines ∧
  shlRefines ∧ shrRefines ∧ rolRefines ∧ rorRefines ∧
  compareRefines ∧ selRefines ∧
  convertRefines ∧ minListRefines ∧ maxListRefines ∧ limitRefines ∧
  muxRefines ∧ lenRefines ∧ lenMaxRefines ∧ lenCodeUnitRefines

/-- The bundled statement is proved from the per-operation theorems. -/
theorem exact_kernel_refines : exactKernelRefines :=
  ⟨modulus_refines, minimum_refines, maximum_refines, fits_refines,
   mask_refines, count_refines, checked_refines,
   add_refines, sub_refines, mul_refines, neg_refines, div_refines,
   modulo_refines, abs_refines,
   bitAnd_refines, bitOr_refines, bitXor_refines, bitNot_refines,
   shl_refines, shr_refines, rol_refines, ror_refines,
   compare_refines, sel_refines,
   convert_refines, min_list_refines, max_list_refines, limit_refines,
   mux_refines, len_refines, len_max_refines, len_code_unit_refines⟩

end Stc.Bridge.Exact
