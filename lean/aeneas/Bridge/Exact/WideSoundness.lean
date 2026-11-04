/-
Refinement proofs for the operations added by the widened generated model.

This module is compiled only in the non-default `lean/aeneas` package. It
proves the eight propositions stated in `Bridge/Exact/WideRefinement.lean`
against the widened generated model, using the string TCB assumptions in
`Bridge/Exact/WideExternals.lean`.

No proof holes are used.
-/

import Bridge.Exact.WideRefinement
import Bridge.Exact.Soundness

open Aeneas Aeneas.Std

namespace Stc.Exact

/-- `convert` to any one-bit layout tests the value against zero. -/
theorem IntType.convert_to_bool_width {src tgt : IntType} (h : tgt.width = 1) (v : Int) :
    src.convert tgt v = .ok (if v == 0 then 0 else 1) := by
  unfold IntType.convert
  rw [ite_eq_left (by simp [h])]

/-- Same-width signed-to-unsigned conversion of a non-negative value keeps it. -/
theorem IntType.convert_signed_to_unsigned_nonneg {src tgt : IntType} {v : Int}
    (hbool : tgt.width ≠ 1) (hw : src.width = tgt.width)
    (hs : src.signed = true) (ht : tgt.signed = false) (hv : 0 ≤ v) :
    src.convert tgt v = .ok v := by
  unfold IntType.convert
  rw [ite_eq_right (by simp [hbool])]
  rw [ite_eq_right (by omega), ite_eq_right (by omega)]
  rw [ite_eq_right (by simp [hs, ht])]
  rw [ite_eq_right (by simp [ht])]
  rw [ite_eq_right (by omega)]

end Stc.Exact

namespace Stc.Bridge.Exact

/-! ## Turning result refinements into weakest-precondition specs -/

/-- A `Result Std.Usize` spec yields the natural-number result refinement. -/
theorem ResultRefinesUsize.of_spec {r : Result Std.Usize} {expected : Nat}
    (h : r ⦃ v => expected = v.val ⦄) : ResultRefinesUsize r expected := by
  unfold ResultRefinesUsize
  cases r using Result.cases with
  | ret v => simpa using h
  | vis i k => simp at h
  | div => simp at h

/-- An optional-integer result spec yields the option refinement. -/
theorem ResultRefinesOptionInt.of_spec {r : Result (Option Std.I128)}
    {expected : Option Int}
    (h : r ⦃ o =>
      match o with
      | none => expected = none
      | some v => expected = some v.val ⦄) : ResultRefinesOptionInt r expected := by
  unfold ResultRefinesOptionInt
  cases r using Result.cases with
  | ret v =>
      cases v with
      | none => simpa using h
      | some v => simpa using h
  | vis i k => simp at h
  | div => simp at h

/-- A `Result Std.Usize` refinement yields a weakest-precondition spec. -/
theorem ResultRefinesUsize.to_spec {r : Result Std.Usize} {expected : Nat}
    (h : ResultRefinesUsize r expected) : r ⦃ v => v.val = expected ⦄ := by
  unfold ResultRefinesUsize at h
  cases r using Result.cases with
  | ret v => simpa using h.symm
  | vis i k => simp at h
  | div => simp at h

/-- A `Result Bool` refinement yields a weakest-precondition spec. -/
theorem ResultRefinesBool.to_spec {r : Result Bool} {expected : Bool}
    (h : ResultRefinesBool r expected) : r ⦃ v => v = expected ⦄ := by
  unfold ResultRefinesBool at h
  cases r using Result.cases with
  | ret v => simpa using h.symm
  | vis i k => simp at h
  | div => simp at h

/-! ## String external specs -/

/-- `str::len` returns the stored byte length. -/
@[step] theorem strLen_spec (text : Str) :
    exact.core.str.Str.len text ⦃ v => v.val = text.val.length ⦄ :=
  ResultRefinesUsize.to_spec (strLen_refines text)

/-- `str::is_ascii` tests every stored byte. -/
@[step] theorem strIsAscii_spec (text : Str) :
    exact.core.str.Str.is_ascii text ⦃ b => b = isAsciiBytes text.val ⦄ :=
  ResultRefinesBool.to_spec (strIsAscii_refines text)

/-- `str::chars().count()` counts the decoded scalars. -/
@[step] theorem strCharsCount_spec (text : Str) :
    (do let chars ← core.str.Str.chars text
        exact.core.str.iter.Chars.Insts.CoreIterTraitsIteratorIteratorChar.count chars) ⦃
      n => n.val = (strScalars text).length ⦄ :=
  ResultRefinesUsize.to_spec (strCharsCount_refines text)

/-- `str::encode_utf16().count()` counts the UTF-16 code units. -/
@[step] theorem strEncodeUtf16Count_spec (text : Str) :
    (do let units ← exact.core.str.Str.encode_utf16 text
        exact.core.str.iter.EncodeUtf16.Insts.CoreIterTraitsIteratorIteratorU16.count units) ⦃
      n => n.val = Stc.Exact.utf16Length (strScalars text) ⦄ :=
  ResultRefinesUsize.to_spec (strEncodeUtf16Count_refines text)

/-! ## `limit` -/

/-- The generated `limit` refines the exact `LIMIT`. -/
theorem limit_refines : limitRefines := by
  intro mn x mx
  unfold ResultRefinesInt exact.limit Stc.Exact.limit
  by_cases h1 : x < mn
  · have h1' : x.val < mn.val := (IScalar.lt_equiv x mn).mp h1
    by_cases h2 : mx < x
    · have h2' : mx.val < x.val := (IScalar.lt_equiv mx x).mp h2
      simp [bind_ok, h1, h2, h2']
    · have h2' : ¬ mx.val < x.val := fun h => h2 ((IScalar.lt_equiv mx x).mpr h)
      simp [h1, h1', h2, h2']
  · have h1' : ¬ x.val < mn.val := fun h => h1 ((IScalar.lt_equiv x mn).mpr h)
    by_cases h2 : mx < x
    · have h2' : mx.val < x.val := (IScalar.lt_equiv mx x).mp h2
      simp [bind_ok, h1, h2, h2']
    · have h2' : ¬ mx.val < x.val := fun h => h2 ((IScalar.lt_equiv mx x).mpr h)
      simp [h1, h1', h2, h2']

/-! ## `mux` -/

/-- The value of an `usize`-to-`i128` heterogeneous cast. -/
theorem hcast_usize_i128_val (i : Std.Usize) :
    (UScalar.hcast IScalarTy.I128 i).val = (i.val : Int) := by
  have hspec :=
    UScalar.hcast_inBounds_spec (src_ty := UScalarTy.Usize) (tgt_ty := IScalarTy.I128) i
      (by
        have h1 : i.val ≤ UScalar.max UScalarTy.Usize := by scalar_tac
        have h2 : UScalar.max UScalarTy.Usize ≤ IScalar.max IScalarTy.I128 := by
          rw [UScalar.max_USize_eq, IScalar.max_IScalarTy_I128_eq]
          simp only [Usize.max, I128.max, Usize.numBits, I128.numBits,
            UScalarTy.Usize_numBits_eq, IScalarTy.I128_numBits_eq]
          rcases System.Platform.numBits_eq with hbits | hbits <;>
            rw [hbits] <;> norm_num
        omega)
  simpa [lift, WP.spec_ok] using hspec

/-- The value of an `i128`-to-`usize` heterogeneous cast inside the `usize`
range. -/
theorem hcast_i128_usize_val (k : Std.I128) (h0 : 0 ≤ k.val)
    (hmax : k.val ≤ (UScalar.max UScalarTy.Usize : Int)) :
    ((IScalar.hcast UScalarTy.Usize k).val : Int) = k.val := by
  have hspec :=
    IScalar.hcast_inBounds_spec (src_ty := IScalarTy.I128) (tgt_ty := UScalarTy.Usize) k
      ⟨h0, hmax⟩
  simpa [lift, WP.spec_ok] using hspec

/-- The generated `mux` refines the exact `MUX`. -/
theorem mux_refines : muxRefines := by
  intro k values default
  unfold ResultRefinesInt
  by_cases hk : k < 0#i128
  · have hk' : k.val < (0#i128).val := (IScalar.lt_equiv k 0#i128).mp hk
    have h0 : (0#i128).val = 0 := by simp
    have hnneg : ¬ (0 ≤ k.val) := by omega
    unfold exact.mux
    simp [hk, hnneg, Stc.Exact.mux]
  · have hk' : ¬ k.val < (0#i128).val := (IScalar.lt_equiv k 0#i128).not.mp hk
    have h0 : (0#i128).val = 0 := by simp
    have hk0 : 0 ≤ k.val := by omega
    have hmux : exact.mux k values default =
        if k ≥ UScalar.hcast IScalarTy.I128 (Slice.len values) then
          .ok default
        else Slice.index_usize values (IScalar.hcast UScalarTy.Usize k) := by
      unfold exact.mux
      simp only [hk, ite_false, lift, bind_ok]
    by_cases hcmp : k ≥ UScalar.hcast IScalarTy.I128 (Slice.len values)
    · -- selector past the end
      have hle : (values.length : Int) ≤ k.val := by
        have h := (IScalar.le_equiv _ _).mp hcmp
        rw [hcast_usize_i128_val, Slice.len_val] at h
        exact h
      have hpast : (values.val.map (fun v => v.val)).length ≤ k.val.toNat := by
        rw [List.length_map]
        exact (Int.le_toNat hk0).mpr hle
      rw [hmux, ite_eq_left hcmp, Result.match.ok, Stc.Exact.mux_past_end hk0 hpast]
    · have hlt : k.val < (values.length : Int) := by
        have h := (IScalar.le_equiv _ _).not.mp hcmp
        rw [hcast_usize_i128_val, Slice.len_val] at h
        omega
      have hlen : k.val.toNat < (values.val.map (fun v => v.val)).length := by
        rw [List.length_map]
        exact (Int.toNat_lt hk0).mpr hlt
      have hmax : k.val ≤ (UScalar.max UScalarTy.Usize : Int) := by
        have h1 : values.length ≤ Usize.max := values.property
        have h2 : (values.length : Int) ≤ (Usize.max : Int) := by exact_mod_cast h1
        have h3 : k.val < (values.length : Int) := hlt
        have h4 : (UScalar.max UScalarTy.Usize : Int) = (Usize.max : Int) := by
          rw [UScalar.max_USize_eq]
        omega
      have hi2 : ((IScalar.hcast UScalarTy.Usize k).val : Int) = k.val :=
        hcast_i128_usize_val k hk0 hmax
      have hi2nat : (IScalar.hcast UScalarTy.Usize k).val = k.val.toNat := by
        have h := hi2
        omega
      have hbound : (IScalar.hcast UScalarTy.Usize k).val < values.length := by
        rw [hi2nat]
        exact (Int.toNat_lt hk0).mpr hlt
      have hspec :=
        Slice.index_usize_spec values (IScalar.hcast UScalarTy.Usize k) hbound
      rw [hmux, ite_eq_right hcmp]
      cases h : Slice.index_usize values (IScalar.hcast UScalarTy.Usize k) using Result.cases with
      | ret v =>
          rw [h] at hspec
          simp only [WP.spec_ok] at hspec
          show Stc.Exact.mux k.val (List.map (fun v => v.val) values.val) default.val = v.val
          rw [hspec, Stc.Exact.mux_in_range hk0 hlen]
          simp only [List.getElem_map, hi2nat]
      | vis eff cont =>
          rw [h] at hspec
          simp at hspec
      | div =>
          rw [h] at hspec
          simp at hspec

/-! ## `min_list` and `max_list` -/

/-- Appending one element updates the minimum to `min acc x`. -/
theorem min?_append_singleton {ms : List Int} {acc x : Int}
    (h : ms.min? = some acc) :
    (ms ++ [x]).min? = some (min acc x) := by
  induction ms with
  | nil => simp at h
  | cons y ys ih =>
      rw [List.cons_append, List.min?_cons']
      rw [List.min?_cons'] at h
      rw [List.foldl_append]
      simp only [List.foldl_cons, List.foldl_nil]
      rw [Option.some.inj h]

/-- Appending one element updates the maximum to `max acc x`. -/
theorem max?_append_singleton {ms : List Int} {acc x : Int}
    (h : ms.max? = some acc) :
    (ms ++ [x]).max? = some (max acc x) := by
  induction ms with
  | nil => simp at h
  | cons y ys ih =>
      rw [List.cons_append, List.max?_cons']
      rw [List.max?_cons'] at h
      rw [List.foldl_append]
      simp only [List.foldl_cons, List.foldl_nil]
      rw [Option.some.inj h]

/-- Loop invariant for the generated `min_list` loop. -/
def minInv (values : Slice Std.I128) (acc : Std.I128) (index : Std.Usize) : Prop :=
  1 ≤ index.val ∧ index.val ≤ values.length ∧
    Stc.Exact.minList ((values.val.take index.val).map (fun v => v.val)) = some acc.val

/-- Final result predicate for the generated `min_list` loop. -/
def minPost (values : Slice Std.I128) (acc : Std.I128) : Prop :=
  Stc.Exact.minList (values.val.map (fun v => v.val)) = some acc.val

/-- Loop invariant for the generated `max_list` loop. -/
def maxInv (values : Slice Std.I128) (acc : Std.I128) (index : Std.Usize) : Prop :=
  1 ≤ index.val ∧ index.val ≤ values.length ∧
    Stc.Exact.maxList ((values.val.take index.val).map (fun v => v.val)) = some acc.val

/-- Final result predicate for the generated `max_list` loop. -/
def maxPost (values : Slice Std.I128) (acc : Std.I128) : Prop :=
  Stc.Exact.maxList (values.val.map (fun v => v.val)) = some acc.val

/-- The `min_list` loop body preserves the invariant and decreases the measure. -/
theorem min_list_loop_body_spec (values : Slice Std.I128) (acc : Std.I128) (index : Std.Usize)
    (hinv : minInv values acc index) :
    exact.min_list_loop.body values acc index ⦃ r =>
      match r with
      | .done y => minPost values y
      | .cont x' =>
          minInv values x'.1 x'.2 ∧ values.length - x'.2.val < values.length - index.val ⦄ := by
  unfold exact.min_list_loop.body
  step*
  · -- continue branch
    rename_i hguard
    obtain ⟨h1, h2, hmin⟩ := hinv
    have hltlen : index.val < values.length := by
      have h := (UScalar.lt_equiv index (Slice.len values)).mp hguard
      rwa [Slice.len_val] at h
    have hget : value.val = (values.val[index.val]).val := by
      rw [value_post]
    by_cases hlt : value < acc
    · rw [ite_eq_left hlt]
      simp only [bind_ok]
      have hmax : index.val + (1#usize).val ≤ UScalar.max UScalarTy.Usize := by
        rw [UScalar.max_USize_eq]
        have h1' : (1#usize).val = 1 := by simp
        have hp : values.length ≤ Usize.max := values.property
        omega
      apply WP.spec_bind (UScalar.add_spec (ty := UScalarTy.Usize) (x := index)
        (y := 1#usize) hmax)
      intro index1 hindex1
      simp only [WP.spec_ok]
      have hidx1 : index1.val = index.val + 1 := by
        have h := hindex1
        have h1' : (1#usize).val = 1 := by simp
        omega
      show minInv values value index1 ∧
        values.length - index1.val < values.length - index.val
      refine ⟨⟨by omega, by omega, ?_⟩, by omega⟩
      have htake := List.take_succ_eq_append_getElem (l := values.val) (i := index.val) hltlen
      have hmap : (values.val.take (index.val + 1)).map (fun v => v.val) =
          (values.val.take index.val).map (fun v => v.val) ++ [value.val] := by
        rw [htake]
        simp only [List.map_append, List.map_cons, List.map_nil]
        rw [hget]
      rw [hidx1, hmap]
      unfold Stc.Exact.minList
      have hmin' : ((values.val.take index.val).map (fun v => v.val)).min? = some acc.val := hmin
      rw [min?_append_singleton hmin']
      have hlt' : value.val < acc.val := (IScalar.lt_equiv value acc).mp hlt
      rw [Int.min_eq_right (le_of_lt hlt')]
    · rw [ite_eq_right hlt]
      simp only [bind_ok]
      have hmax : index.val + (1#usize).val ≤ UScalar.max UScalarTy.Usize := by
        rw [UScalar.max_USize_eq]
        have h1' : (1#usize).val = 1 := by simp
        have hp : values.length ≤ Usize.max := values.property
        omega
      apply WP.spec_bind (UScalar.add_spec (ty := UScalarTy.Usize) (x := index)
        (y := 1#usize) hmax)
      intro index1 hindex1
      simp only [WP.spec_ok]
      have hidx1 : index1.val = index.val + 1 := by
        have h := hindex1
        have h1' : (1#usize).val = 1 := by simp
        omega
      show minInv values acc index1 ∧
        values.length - index1.val < values.length - index.val
      refine ⟨⟨by omega, by omega, ?_⟩, by omega⟩
      have htake := List.take_succ_eq_append_getElem (l := values.val) (i := index.val) hltlen
      have hmap : (values.val.take (index.val + 1)).map (fun v => v.val) =
          (values.val.take index.val).map (fun v => v.val) ++ [value.val] := by
        rw [htake]
        simp only [List.map_append, List.map_cons, List.map_nil]
        rw [hget]
      rw [hidx1, hmap]
      unfold Stc.Exact.minList
      have hmin' : ((values.val.take index.val).map (fun v => v.val)).min? = some acc.val := hmin
      rw [min?_append_singleton hmin']
      have hlt' : ¬ value.val < acc.val := fun hv => hlt ((IScalar.lt_equiv value acc).mpr hv)
      rw [Int.min_eq_left (by omega)]
  · -- done branch
    rename_i hguard
    obtain ⟨h1, h2, hmin⟩ := hinv
    have hle : values.length ≤ index.val := by
      have h := (UScalar.lt_equiv index (Slice.len values)).not.mp hguard
      rw [Slice.len_val] at h
      omega
    unfold minPost
    rw [List.take_of_length_le (l := values.val) hle] at hmin
    exact hmin

/-- The `max_list` loop body preserves the invariant and decreases the measure. -/
theorem max_list_loop_body_spec (values : Slice Std.I128) (acc : Std.I128) (index : Std.Usize)
    (hinv : maxInv values acc index) :
    exact.max_list_loop.body values acc index ⦃ r =>
      match r with
      | .done y => maxPost values y
      | .cont x' =>
          maxInv values x'.1 x'.2 ∧ values.length - x'.2.val < values.length - index.val ⦄ := by
  unfold exact.max_list_loop.body
  step*
  · -- continue branch
    rename_i hguard
    obtain ⟨h1, h2, hmax⟩ := hinv
    have hltlen : index.val < values.length := by
      have h := (UScalar.lt_equiv index (Slice.len values)).mp hguard
      rwa [Slice.len_val] at h
    have hget : value.val = (values.val[index.val]).val := by
      rw [value_post]
    by_cases hlt : acc < value
    · rw [ite_eq_left hlt]
      simp only [bind_ok]
      have hbound : index.val + (1#usize).val ≤ UScalar.max UScalarTy.Usize := by
        rw [UScalar.max_USize_eq]
        have h1' : (1#usize).val = 1 := by simp
        have hp : values.length ≤ Usize.max := values.property
        omega
      apply WP.spec_bind (UScalar.add_spec (ty := UScalarTy.Usize) (x := index)
        (y := 1#usize) hbound)
      intro index1 hindex1
      simp only [WP.spec_ok]
      have hidx1 : index1.val = index.val + 1 := by
        have h := hindex1
        have h1' : (1#usize).val = 1 := by simp
        omega
      show maxInv values value index1 ∧
        values.length - index1.val < values.length - index.val
      refine ⟨⟨by omega, by omega, ?_⟩, by omega⟩
      have htake := List.take_succ_eq_append_getElem (l := values.val) (i := index.val) hltlen
      have hmap : (values.val.take (index.val + 1)).map (fun v => v.val) =
          (values.val.take index.val).map (fun v => v.val) ++ [value.val] := by
        rw [htake]
        simp only [List.map_append, List.map_cons, List.map_nil]
        rw [hget]
      rw [hidx1, hmap]
      unfold Stc.Exact.maxList
      have hmax' : ((values.val.take index.val).map (fun v => v.val)).max? = some acc.val := hmax
      rw [max?_append_singleton hmax']
      have hlt' : acc.val < value.val := (IScalar.lt_equiv acc value).mp hlt
      rw [Int.max_eq_right (le_of_lt hlt')]
    · rw [ite_eq_right hlt]
      simp only [bind_ok]
      have hbound : index.val + (1#usize).val ≤ UScalar.max UScalarTy.Usize := by
        rw [UScalar.max_USize_eq]
        have h1' : (1#usize).val = 1 := by simp
        have hp : values.length ≤ Usize.max := values.property
        omega
      apply WP.spec_bind (UScalar.add_spec (ty := UScalarTy.Usize) (x := index)
        (y := 1#usize) hbound)
      intro index1 hindex1
      simp only [WP.spec_ok]
      have hidx1 : index1.val = index.val + 1 := by
        have h := hindex1
        have h1' : (1#usize).val = 1 := by simp
        omega
      show maxInv values acc index1 ∧
        values.length - index1.val < values.length - index.val
      refine ⟨⟨by omega, by omega, ?_⟩, by omega⟩
      have htake := List.take_succ_eq_append_getElem (l := values.val) (i := index.val) hltlen
      have hmap : (values.val.take (index.val + 1)).map (fun v => v.val) =
          (values.val.take index.val).map (fun v => v.val) ++ [value.val] := by
        rw [htake]
        simp only [List.map_append, List.map_cons, List.map_nil]
        rw [hget]
      rw [hidx1, hmap]
      unfold Stc.Exact.maxList
      have hmax' : ((values.val.take index.val).map (fun v => v.val)).max? = some acc.val := hmax
      rw [max?_append_singleton hmax']
      have hlt' : ¬ acc.val < value.val := fun hv => hlt ((IScalar.lt_equiv acc value).mpr hv)
      rw [Int.max_eq_left (by omega)]
  · -- done branch
    rename_i hguard
    obtain ⟨h1, h2, hmax⟩ := hinv
    have hle : values.length ≤ index.val := by
      have h := (UScalar.lt_equiv index (Slice.len values)).not.mp hguard
      rw [Slice.len_val] at h
      omega
    unfold maxPost
    rw [List.take_of_length_le (l := values.val) hle] at hmax
    exact hmax

/-- The generated `min_list` loop returns the list minimum. -/
theorem min_list_loop_spec (values : Slice Std.I128) (acc : Std.I128) (index : Std.Usize)
    (hinv : minInv values acc index) :
    exact.min_list_loop values acc index ⦃ acc' => minPost values acc' ⦄ := by
  unfold exact.min_list_loop
  apply loop.spec_decr_nat
    (measure := fun p : Std.I128 × Std.Usize => values.length - p.2.val)
    (inv := fun p => minInv values p.1 p.2)
    (post := fun acc' => minPost values acc')
  · intro p hp
    obtain ⟨a, i⟩ := p
    apply WP.spec_mono (min_list_loop_body_spec values a i hp)
    intro r hr
    cases r with
    | done y => exact hr
    | cont x' => exact hr
  · exact hinv

/-- The generated `max_list` loop returns the list maximum. -/
theorem max_list_loop_spec (values : Slice Std.I128) (acc : Std.I128) (index : Std.Usize)
    (hinv : maxInv values acc index) :
    exact.max_list_loop values acc index ⦃ acc' => maxPost values acc' ⦄ := by
  unfold exact.max_list_loop
  apply loop.spec_decr_nat
    (measure := fun p : Std.I128 × Std.Usize => values.length - p.2.val)
    (inv := fun p => maxInv values p.1 p.2)
    (post := fun acc' => maxPost values acc')
  · intro p hp
    obtain ⟨a, i⟩ := p
    apply WP.spec_mono (max_list_loop_body_spec values a i hp)
    intro r hr
    cases r with
    | done y => exact hr
    | cont x' => exact hr
  · exact hinv

/-- The generated `min_list` returns the exact `MIN`. -/
@[step] theorem min_list_spec (values : Slice Std.I128) :
    exact.min_list values ⦃ o =>
      match o with
      | none => Stc.Exact.minList (values.val.map (fun v => v.val)) = none
      | some v => Stc.Exact.minList (values.val.map (fun v => v.val)) = some v.val ⦄ := by
  unfold exact.min_list
  step*
  · -- empty branch
    rename_i htrue
    have hempty : values.length = 0 := by
      rwa [b_post] at htrue
    have hnil : values.val = [] := List.length_eq_zero_iff.mp hempty
    simp [Stc.Exact.minList, hnil]
  · -- nonempty branch
    rename_i hstep hfalse
    have hne : values.length ≠ 0 := by
      rw [b_post] at hfalse
      exact hfalse
    have hpos : 0 < values.length := Nat.pos_of_ne_zero hne
    have hbound : (0#usize).val < values.length := by simpa using hpos
    have hinv : minInv values acc 1#usize := by
      have hval1 : (1#usize).val = 1 := by simp
      refine ⟨by simp, by omega, ?_⟩
      have htake := List.take_succ_eq_append_getElem (l := values.val) (i := 0)
        (by simpa using hbound)
      have hlist : (values.val.take (1#usize).val).map (fun v => v.val) = [acc.val] := by
        rw [hval1, htake]
        simp only [List.take_zero, List.nil_append, List.map_cons, List.map_nil]
        rw [acc_post]
      rw [hlist]
      simp [Stc.Exact.minList]
    apply WP.spec_bind (k := fun acc1 => Result.ok (some acc1))
      (min_list_loop_spec values acc 1#usize hinv)
    intro acc1 hacc1
    simp only [WP.spec_ok]
    exact hacc1

/-- The generated `max_list` returns the exact `MAX`. -/
@[step] theorem max_list_spec (values : Slice Std.I128) :
    exact.max_list values ⦃ o =>
      match o with
      | none => Stc.Exact.maxList (values.val.map (fun v => v.val)) = none
      | some v => Stc.Exact.maxList (values.val.map (fun v => v.val)) = some v.val ⦄ := by
  unfold exact.max_list
  step*
  · -- empty branch
    rename_i htrue
    have hempty : values.length = 0 := by
      rwa [b_post] at htrue
    have hnil : values.val = [] := List.length_eq_zero_iff.mp hempty
    simp [Stc.Exact.maxList, hnil]
  · -- nonempty branch
    rename_i hstep hfalse
    have hne : values.length ≠ 0 := by
      rw [b_post] at hfalse
      exact hfalse
    have hpos : 0 < values.length := Nat.pos_of_ne_zero hne
    have hbound : (0#usize).val < values.length := by simpa using hpos
    have hinv : maxInv values acc 1#usize := by
      have hval1 : (1#usize).val = 1 := by simp
      refine ⟨by simp, by omega, ?_⟩
      have htake := List.take_succ_eq_append_getElem (l := values.val) (i := 0)
        (by simpa using hbound)
      have hlist : (values.val.take (1#usize).val).map (fun v => v.val) = [acc.val] := by
        rw [hval1, htake]
        simp only [List.take_zero, List.nil_append, List.map_cons, List.map_nil]
        rw [acc_post]
      rw [hlist]
      simp [Stc.Exact.maxList]
    apply WP.spec_bind (k := fun acc1 => Result.ok (some acc1))
      (max_list_loop_spec values acc 1#usize hinv)
    intro acc1 hacc1
    simp only [WP.spec_ok]
    exact hacc1

/-- The generated `min_list` refines the exact `MIN`. -/
theorem min_list_refines : minListRefines :=
  fun values => ResultRefinesOptionInt.of_spec (min_list_spec values)

/-- The generated `max_list` refines the exact `MAX`. -/
theorem max_list_refines : maxListRefines :=
  fun values => ResultRefinesOptionInt.of_spec (max_list_spec values)

/-! ## String lengths -/

/-- The generated `len` refines the exact `LEN`. -/
@[step] theorem len_spec (kind : exact.StringKind) (text : Str)
    (hguard : kind = exact.StringKind.Sbcs → isAsciiBytes text.val = true) :
    exact.len kind text ⦃ n =>
      n.val = Stc.Exact.len (decodeStringKind kind) (decodedScalars kind text) ⦄ := by
  unfold exact.len
  cases kind
  · simp only [decodeStringKind, decodedScalars, Stc.Exact.len, List.length_map]
    have hascii : isAsciiBytes text.val = true := hguard rfl
    apply WP.spec_bind (strIsAscii_spec text)
    intro b hb
    rw [hb, hascii]
    simp only [ite_true]
    simpa [List.length_map] using strLen_spec text
  · simp only [decodeStringKind, decodedScalars, Stc.Exact.len]
    simpa using strEncodeUtf16Count_spec text
  · simp only [decodeStringKind, decodedScalars, Stc.Exact.len]
    simpa using strCharsCount_spec text

/-- The generated `len_code_unit` refines the exact `LEN_CODE_UNIT`. -/
@[step] theorem len_code_unit_spec (kind : exact.StringKind) (text : Str)
    (hguard : kind = exact.StringKind.Sbcs → isAsciiBytes text.val = true) :
    exact.len_code_unit kind text ⦃ n =>
      n.val = Stc.Exact.lenCodeUnit (decodeStringKind kind) (decodedScalars kind text) ⦄ := by
  unfold exact.len_code_unit
  cases kind
  · simp only [decodeStringKind, decodedScalars, Stc.Exact.lenCodeUnit, List.length_map]
    have hascii : isAsciiBytes text.val = true := hguard rfl
    apply WP.spec_bind (strIsAscii_spec text)
    intro b hb
    rw [hb, hascii]
    simp only [ite_true]
    simpa [List.length_map] using strLen_spec text
  · simp only [decodeStringKind, decodedScalars, Stc.Exact.lenCodeUnit]
    simpa using strEncodeUtf16Count_spec text
  · simp only [decodeStringKind, decodedScalars, Stc.Exact.lenCodeUnit]
    simpa [strScalars_utf8ByteLength] using strLen_spec text

/-! ## `len_max` -/

/-- The generated `len_max` refines the exact `LEN_MAX`. -/
theorem len_max_refines : lenMaxRefines := by
  intro capacity
  unfold ResultRefinesUsize exact.len_max Stc.Exact.lenMax
  simp

/-- The generated `len` refines the exact `LEN`. -/
theorem len_refines : lenRefines := by
  intro kind text hguard
  exact ResultRefinesUsize.of_spec (by simpa [eq_comm] using len_spec kind text hguard)

/-- The generated `len_code_unit` refines the exact `LEN_CODE_UNIT`. -/
theorem len_code_unit_refines : lenCodeUnitRefines := by
  intro kind text hguard
  exact ResultRefinesUsize.of_spec (by simpa [eq_comm] using len_code_unit_spec kind text hguard)

/-! ## `cast_plan` equations -/

/-- `cast_plan` selects `BoolTest` for a one-bit destination. -/
theorem cast_plan_eq_bool {src dst : exact.IntType} (h : dst.width = 1#u8) :
    exact.cast_plan src dst = Result.ok exact.CastPlan.BoolTest := by
  unfold exact.cast_plan
  rw [ite_eq_left h]

/-- `cast_plan` selects `Widen` for a strictly narrower source. -/
theorem cast_plan_eq_widen {src dst : exact.IntType}
    (h1 : ¬ dst.width = 1#u8) (h2 : src.width < dst.width) :
    exact.cast_plan src dst = Result.ok (exact.CastPlan.Widen src.signed) := by
  unfold exact.cast_plan
  rw [ite_eq_right h1, ite_eq_left h2]

/-- `cast_plan` selects `Narrow` for a strictly wider source. -/
theorem cast_plan_eq_narrow {src dst : exact.IntType}
    (h1 : ¬ dst.width = 1#u8) (h2 : ¬ src.width < dst.width)
    (h3 : dst.width < src.width) :
    exact.cast_plan src dst = Result.ok exact.CastPlan.Narrow := by
  unfold exact.cast_plan
  rw [ite_eq_right h1, ite_eq_right h2, ite_eq_left h3]

/-- `cast_plan` selects `Identity` for equal widths and signedness. -/
theorem cast_plan_eq_identity {src dst : exact.IntType}
    (h1 : ¬ dst.width = 1#u8) (h2 : ¬ src.width < dst.width)
    (h3 : ¬ dst.width < src.width) (h4 : src.signed = dst.signed) :
    exact.cast_plan src dst = Result.ok exact.CastPlan.Identity := by
  unfold exact.cast_plan
  rw [ite_eq_right h1, ite_eq_right h2, ite_eq_right h3, ite_eq_left h4]

/-- `cast_plan` selects `Reinterpret` for equal widths and differing signedness. -/
theorem cast_plan_eq_reinterpret {src dst : exact.IntType}
    (h1 : ¬ dst.width = 1#u8) (h2 : ¬ src.width < dst.width)
    (h3 : ¬ dst.width < src.width) (h4 : ¬ src.signed = dst.signed) :
    exact.cast_plan src dst = Result.ok exact.CastPlan.Reinterpret := by
  unfold exact.cast_plan
  rw [ite_eq_right h1, ite_eq_right h2, ite_eq_right h3, ite_eq_right h4]

/-! ## Exact-side helper lemmas -/

/-- The generated `i128` `bool` conversion returns zero or one. -/
@[step] theorem fromI128Bool_spec (value : Std.I128) :
    lift (core.convert.num.FromI128Bool.from (value != 0#i128)) ⦃ i =>
      i.val = (if value.val == 0 then (0 : Int) else 1) ⦄ := by
  unfold lift
  simp only [WP.spec_ok, core.convert.num.FromI128Bool.from]
  by_cases h : value.val = 0
  · have hb : value = 0#i128 := by
      apply IScalar.eq_imp
      simpa using h
    simp [hb]
  · have hb : value ≠ 0#i128 := by
      intro hEq
      exact h (by simpa using IScalar.val_eq_of_eq hEq)
    simp [hb, h]

/-- The generated conversion refines the exact conversion model. -/
@[step] theorem convert_spec (src dst : exact.IntType) (value : Std.I128)
    (_hwS : WellFormed src) (hwD : WellFormed dst) :
    exact.convert src dst value ⦃ r =>
      CoreResultRefines r ((decodeIntType src).convert (decodeIntType dst) value.val) ⦄ := by
  have hwDpos : 0 < dst.width.val := hwD.1
  have hwD64 : dst.width.val ≤ 64 := hwD.2
  unfold exact.convert
  by_cases h1 : dst.width.val = 1
  · -- one-bit destination: the generated code tests against zero
    have hb : dst.width = 1#u8 := UScalar.eq_of_val_eq (by simpa using h1)
    rw [cast_plan_eq_bool hb]
    simp only [bind_ok]
    step*
    simp only [CoreResultRefines]
    rw [Stc.Exact.IntType.convert_to_bool_width (h := by
      simpa [decodeIntType, IntTypeEncoding.toExact] using h1)]
    rw [i_post]
  · have hnb : ¬ dst.width = 1#u8 := fun h => h1 (by simpa using congrArg UScalar.val h)
    by_cases hlt : src.width.val < dst.width.val
    · -- widening conversion
      have hlt' : src.width < dst.width := (UScalar.lt_equiv src.width dst.width).mpr hlt
      rw [cast_plan_eq_widen hnb hlt']
      simp only [bind_ok]
      by_cases hs : src.signed = true
      · rw [ite_eq_left hs]
        by_cases hv : value < 0#i128
        · rw [ite_eq_left hv]
          by_cases ht : dst.signed = true
          · -- signed negative value to signed target: preserved
            rw [ite_eq_left ht]
            simp only [WP.spec_ok, CoreResultRefines]
            rw [Stc.Exact.IntType.convert_widen_signed
              (hbool := by simpa [decodeIntType, IntTypeEncoding.toExact] using h1)
              (h := by simpa [decodeIntType, IntTypeEncoding.toExact] using hlt)
              (hs := by simpa [decodeIntType, IntTypeEncoding.toExact] using hs)
              (ht := by simpa [decodeIntType, IntTypeEncoding.toExact] using ht)
              (hv := by simpa using (IScalar.lt_equiv value 0#i128).mp hv)]
          · -- signed negative value to unsigned target: add the modulus
            rw [ite_eq_right ht]
            apply WP.spec_bind
              (k := fun (i : Std.I128) => (do
                let i1 ← value + i
                Result.ok (core.result.Result.Ok i1)))
              (modulus_spec dst hwD)
            intro i hi
            have hiw : i.val = (2 : Int) ^ dst.width.val := by
              rw [← hi]
              simp [decodeIntType, IntTypeEncoding.toExact, Stc.Exact.IntType.modulus]
            have hpos : 0 < i.val := by
              rw [hiw]; positivity
            have hvlt : value.val < 0 := by
              simpa using (IScalar.lt_equiv value 0#i128).mp hv
            have hvmin : I128.min ≤ value.val := by
              simpa [I128.min, I128.numBits] using IScalar.hmin value
            have hvmax : value.val ≤ I128.max := by
              have h := IScalar.hmax value
              simp [I128.max, I128.numBits] at h ⊢
              omega
            have hpow : (2 : Int) ^ dst.width.val ≤ (2 : Int) ^ 64 := by
              exact_mod_cast Nat.pow_le_pow_right (by norm_num) hwD.2
            have h64max : (2 : Int) ^ 64 ≤ I128.max := by
              simp [I128.max, I128.numBits]
            apply WP.spec_bind
              (k := fun (i1 : Std.I128) => Result.ok (core.result.Result.Ok i1))
              (I128.add_spec (x := value) (y := i)
                (by omega) (by omega))
            intro i1 hi1
            simp only [WP.spec_ok, CoreResultRefines]
            rw [Stc.Exact.IntType.convert_widen_neg_unsigned
              (hbool := by simpa [decodeIntType, IntTypeEncoding.toExact] using h1)
              (h := by simpa [decodeIntType, IntTypeEncoding.toExact] using hlt)
              (hs := by simpa [decodeIntType, IntTypeEncoding.toExact] using hs)
              (ht := by simpa [decodeIntType, IntTypeEncoding.toExact] using ht)
              (hv := hvlt)]
            rw [hi1, hiw]
            simp [decodeIntType, IntTypeEncoding.toExact]
        · -- signed non-negative value: preserved
          rw [ite_eq_right hv]
          simp only [WP.spec_ok, CoreResultRefines]
          rw [Stc.Exact.IntType.convert_widen_value
            (hbool := by simpa [decodeIntType, IntTypeEncoding.toExact] using h1)
            (h := by simpa [decodeIntType, IntTypeEncoding.toExact] using hlt)
            (hv := Or.inr (by
              have h : ¬ value.val < 0 := fun h => hv ((IScalar.lt_equiv value 0#i128).mpr h)
              omega))]
      · -- unsigned source: preserved
        rw [ite_eq_right hs]
        simp only [WP.spec_ok, CoreResultRefines]
        rw [Stc.Exact.IntType.convert_widen_value
          (hbool := by simpa [decodeIntType, IntTypeEncoding.toExact] using h1)
          (h := by simpa [decodeIntType, IntTypeEncoding.toExact] using hlt)
          (hv := Or.inl (by simpa [decodeIntType, IntTypeEncoding.toExact] using hs))]
    · by_cases hgt : dst.width.val < src.width.val
      · -- narrowing conversion
        have hgt' : dst.width < src.width := (UScalar.lt_equiv dst.width src.width).mpr hgt
        have hnlt : ¬ src.width < dst.width := fun h =>
          hlt ((UScalar.lt_equiv src.width dst.width).mp h)
        rw [cast_plan_eq_narrow hnb hnlt hgt']
        simp only [bind_ok]
        apply WP.spec_bind
          (k := fun (b : Bool) => if b
            then Result.ok (core.result.Result.Ok value)
            else Result.ok (core.result.Result.Err exact.Trap.Range))
          (fits_spec dst value hwD)
        intro b hb
        by_cases hbt : b = true
        · rw [ite_eq_left hbt]
          simp only [WP.spec_ok, CoreResultRefines]
          rw [Stc.Exact.IntType.convert_narrow_ok
            (hbool := by simpa [decodeIntType, IntTypeEncoding.toExact] using h1)
            (h := by simpa [decodeIntType, IntTypeEncoding.toExact] using hgt)
            (hf := by rw [hb, hbt])]
        · rw [ite_eq_right hbt]
          simp only [WP.spec_ok, CoreResultRefines, decodeTrap]
          rw [Stc.Exact.IntType.convert_narrow_error
            (hbool := by simpa [decodeIntType, IntTypeEncoding.toExact] using h1)
            (h := by simpa [decodeIntType, IntTypeEncoding.toExact] using hgt)]
          have hbf : (decodeIntType dst).fits value.val = false := by
            rw [hb]
            simpa using hbt
          rw [hbf]
      · have hw : src.width.val = dst.width.val := by omega
        by_cases hsigned : src.signed = dst.signed
        · -- identity
          have hnlt2 : ¬ dst.width < src.width := fun h =>
            hgt ((UScalar.lt_equiv dst.width src.width).mp h)
          rw [cast_plan_eq_identity hnb hlt hnlt2 hsigned]
          simp only [bind_ok]
          simp only [WP.spec_ok, CoreResultRefines]
          rw [Stc.Exact.IntType.convert_same_signedness
            (hbool := by simpa [decodeIntType, IntTypeEncoding.toExact] using h1)
            (hw := by simpa [decodeIntType, IntTypeEncoding.toExact] using hw)
            (hs := by simpa [decodeIntType, IntTypeEncoding.toExact] using hsigned)]
        · -- reinterpretation between signedness
          have hnlt2 : ¬ dst.width < src.width := fun h =>
            hgt ((UScalar.lt_equiv dst.width src.width).mp h)
          rw [cast_plan_eq_reinterpret hnb hlt hnlt2 hsigned]
          simp only [bind_ok]
          by_cases ht : dst.signed = true
          · -- unsigned source, signed target
            rw [ite_eq_left ht]
            have hsfalse : src.signed = false := by
              by_cases hsrc : src.signed = true
              · exact absurd (by rw [hsrc, ht]) hsigned
              · simpa using hsrc
            apply WP.spec_bind
              (k := fun (i : Std.U8) => (do
                let i1 ← 1#i128 <<< i
                if value >= i1
                then
                  let i2 ← exact.IntType.modulus dst
                  let value1 ← value - i2
                  Result.ok (core.result.Result.Ok value1)
                else Result.ok (core.result.Result.Ok value)))
              (UScalar.sub_spec (ty := UScalarTy.U8) (x := dst.width) (y := 1#u8)
                (by
                  have h1v : (1#u8).val = 1 := by simp
                  omega))
            intro i hi
            have hiw : i.val = dst.width.val - 1 := by
              have h1v : (1#u8).val = 1 := by simp
              have h := hi.1
              omega
            apply WP.spec_bind
              (k := fun (i1 : Std.I128) =>
                if value >= i1
                then
                  (do
                    let i2 ← exact.IntType.modulus dst
                    let value1 ← value - i2
                    Result.ok (core.result.Result.Ok value1))
                else Result.ok (core.result.Result.Ok value))
              (I128.ShiftLeft_spec (x := 1#i128) (y := i) (by rw [hiw]; omega))
            intro i1 hi1
            have hi1v : i1.val = (2 : Int) ^ (dst.width.val - 1) := by
              rw [hi1.1]
              have h1v : ((1#i128).val : Int) = 1 := by simp
              rw [h1v, Int.shiftLeft_eq, one_mul, hiw, bmod_two_pow (by omega)]
            by_cases hge : value >= i1
            · -- high bit set: subtract the modulus
              rw [ite_eq_left hge]
              have hge' : (2 : Int) ^ (dst.width.val - 1) ≤ value.val := by
                have h := (IScalar.le_equiv i1 value).mp hge
                rwa [hi1v] at h
              apply WP.spec_bind
                (k := fun (i2 : Std.I128) => (do
                  let value1 ← value - i2
                  Result.ok (core.result.Result.Ok value1)))
                (modulus_spec dst hwD)
              intro i2 hi2
              have hi2v : i2.val = (2 : Int) ^ dst.width.val := by
                rw [← hi2]
                simp [decodeIntType, IntTypeEncoding.toExact, Stc.Exact.IntType.modulus]
              have hvmin : I128.min ≤ value.val := by
                simpa [I128.min, I128.numBits] using IScalar.hmin value
              have hvmax : value.val ≤ I128.max := by
                have h := IScalar.hmax value
                simp [I128.max, I128.numBits] at h ⊢
                omega
              have hpow : (2 : Int) ^ dst.width.val ≤ (2 : Int) ^ 64 := by
                exact_mod_cast Nat.pow_le_pow_right (by norm_num) hwD.2
              have h64 : (2 : Int) ^ 64 ≤ -I128.min := by
                simp [I128.min, I128.numBits]
              have hnonneg : (0 : Int) ≤ value.val := by
                have hp : (0 : Int) < (2 : Int) ^ (dst.width.val - 1) := by positivity
                omega
              apply WP.spec_bind
                (k := fun (value1 : Std.I128) =>
                  Result.ok (core.result.Result.Ok value1))
                (I128.sub_spec (x := value) (y := i2)
                  (by rw [hi2v]; omega) (by rw [hi2v]; omega))
              intro value1 hvalue1
              simp only [WP.spec_ok, CoreResultRefines]
              rw [Stc.Exact.IntType.convert_unsigned_to_signed_high
                (hbool := by simpa [decodeIntType, IntTypeEncoding.toExact] using h1)
                (hw := by simpa [decodeIntType, IntTypeEncoding.toExact] using hw)
                (hs := by simpa [decodeIntType, IntTypeEncoding.toExact] using hsfalse)
                (ht := by simpa [decodeIntType, IntTypeEncoding.toExact] using ht)
                (hv := hge')]
              rw [hvalue1, hi2v]
              simp [decodeIntType, IntTypeEncoding.toExact]
            · -- high bit clear: keep the value
              rw [ite_eq_right hge]
              simp only [WP.spec_ok, CoreResultRefines]
              rw [Stc.Exact.IntType.convert_unsigned_to_signed_low
                (hbool := by simpa [decodeIntType, IntTypeEncoding.toExact] using h1)
                (hw := by simpa [decodeIntType, IntTypeEncoding.toExact] using hw)
                (hs := by simpa [decodeIntType, IntTypeEncoding.toExact] using hsfalse)
                (ht := by simpa [decodeIntType, IntTypeEncoding.toExact] using ht)
                (hv := by
                  have h : ¬ i1.val ≤ value.val := fun hle =>
                    hge ((IScalar.le_equiv i1 value).mpr hle)
                  have hlt : value.val < 2 ^ (dst.width.val - 1) := by
                    rw [← hi1v]
                    omega
                  simpa [decodeIntType, IntTypeEncoding.toExact] using hlt)]
          · -- signed source, unsigned target
            rw [ite_eq_right ht]
            have hstrue : src.signed = true := by
              by_cases hsrc : src.signed = true
              · exact hsrc
              · have hfalse : src.signed = false := by simpa using hsrc
                exfalso
                apply hsigned
                rw [hfalse]
                simpa using ht
            by_cases hv : value < 0#i128
            · -- negative: add the modulus
              rw [ite_eq_left hv]
              have hvlt : value.val < 0 := by
                simpa using (IScalar.lt_equiv value 0#i128).mp hv
              apply WP.spec_bind
                (k := fun (i : Std.I128) => (do
                  let value1 ← value + i
                  Result.ok (core.result.Result.Ok value1)))
                (modulus_spec dst hwD)
              intro i hi
              have hiw : i.val = (2 : Int) ^ dst.width.val := by
                rw [← hi]
                simp [decodeIntType, IntTypeEncoding.toExact, Stc.Exact.IntType.modulus]
              have hvmin : I128.min ≤ value.val := by
                simpa [I128.min, I128.numBits] using IScalar.hmin value
              have hvmax : value.val ≤ I128.max := by
                have h := IScalar.hmax value
                simp [I128.max, I128.numBits] at h ⊢
                omega
              have hpow : (2 : Int) ^ dst.width.val ≤ (2 : Int) ^ 64 := by
                exact_mod_cast Nat.pow_le_pow_right (by norm_num) hwD.2
              have h64max : (2 : Int) ^ 64 ≤ I128.max := by
                simp [I128.max, I128.numBits]
              apply WP.spec_bind
                (k := fun (value1 : Std.I128) =>
                  Result.ok (core.result.Result.Ok value1))
                (I128.add_spec (x := value) (y := i)
                  (by rw [hiw]; omega) (by rw [hiw]; omega))
              intro value1 hvalue1
              simp only [WP.spec_ok, CoreResultRefines]
              rw [Stc.Exact.IntType.convert_signed_to_unsigned
                (hbool := by simpa [decodeIntType, IntTypeEncoding.toExact] using h1)
                (hw := by simpa [decodeIntType, IntTypeEncoding.toExact] using hw)
                (hs := by simpa [decodeIntType, IntTypeEncoding.toExact] using hstrue)
                (ht := by simpa [decodeIntType, IntTypeEncoding.toExact] using ht)
                (hv := hvlt)]
              rw [hvalue1, hiw]
              simp [decodeIntType, IntTypeEncoding.toExact]
            · -- non-negative: keep the value
              rw [ite_eq_right hv]
              simp only [WP.spec_ok, CoreResultRefines]
              rw [Stc.Exact.IntType.convert_signed_to_unsigned_nonneg
                (hbool := by simpa [decodeIntType, IntTypeEncoding.toExact] using h1)
                (hw := by simpa [decodeIntType, IntTypeEncoding.toExact] using hw)
                (hs := by simpa [decodeIntType, IntTypeEncoding.toExact] using hstrue)
                (ht := by simpa [decodeIntType, IntTypeEncoding.toExact] using ht)
                (hv := by
                  have h : ¬ value.val < 0 := fun hlt =>
                    hv ((IScalar.lt_equiv value 0#i128).mpr hlt)
                  omega)]

/-- `convert` refines the exact conversion model. -/
theorem convert_refines : convertRefines := by
  intro src dst value hwS hwD
  exact ResultRefinesExact.of_spec (convert_spec src dst value hwS hwD)

end Stc.Bridge.Exact
