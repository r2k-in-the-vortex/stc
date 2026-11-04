/-
Refinement propositions for the operations added by the widened generated
model: `convert`, `min_list`, `max_list`, `limit`, `mux`, `len`, `len_max`,
and `len_code_unit`.

This module is compiled only in the non-default `lean/aeneas` package. The
string operations read the TCB-modelled scalar sequence from
`Bridge.Exact.StrModel`; `SBCS` data must be ASCII for the non-panicking path,
which is the first argument of `lenRefines` and `lenCodeUnitRefines`.
-/

import Bridge.Exact.WideExternals
import Stc.Exact.Conv
import Stc.Exact.Select
import Stc.Exact.Strings

namespace Stc.Bridge.Exact

open Aeneas Aeneas.Std

/-! ## Conversions -/

/-- The generated `cast_plan`/`convert` pair refines the exact conversion
model. Both layouts are well formed: the generated reinterpret shift is an
`i128` shift, which only agrees with the exact model below 64 bits. -/
def convertRefines : Prop :=
  ∀ (src dst : exact.IntType) (value : Std.I128),
    WellFormed src →
      WellFormed dst →
        ResultRefinesExact (exact.convert src dst value)
          ((decodeIntType src).convert (decodeIntType dst) value.val)

/-! ## Selection over value lists -/

/-- The generated `min_list` refines the exact `MIN`. -/
def minListRefines : Prop :=
  ∀ values : Slice Std.I128,
    ResultRefinesOptionInt (exact.min_list values)
      (Stc.Exact.minList (values.val.map (fun value => value.val)))

/-- The generated `max_list` refines the exact `MAX`. -/
def maxListRefines : Prop :=
  ∀ values : Slice Std.I128,
    ResultRefinesOptionInt (exact.max_list values)
      (Stc.Exact.maxList (values.val.map (fun value => value.val)))

/-- The generated `limit` refines the exact `LIMIT`. -/
def limitRefines : Prop :=
  ∀ (mn x mx : Std.I128),
    ResultRefinesInt (exact.limit mn x mx)
      (Stc.Exact.limit mn.val x.val mx.val)

/-- The generated `mux` refines the exact `MUX`. -/
def muxRefines : Prop :=
  ∀ (k : Std.I128) (values : Slice Std.I128) (default : Std.I128),
    ResultRefinesInt (exact.mux k values default)
      (Stc.Exact.mux k.val (values.val.map (fun value => value.val)) default.val)

/-! ## String lengths -/

/-- The generated `len` refines the exact `LEN`. `SBCS` data must be ASCII:
the generated path panics on non-ASCII bytes, while the exact model counts
bytes. -/
def lenRefines : Prop :=
  ∀ (kind : exact.StringKind) (text : Str),
    (kind = exact.StringKind.Sbcs → isAsciiBytes text.val = true) →
      ResultRefinesUsize (exact.len kind text)
        (Stc.Exact.len (decodeStringKind kind) (decodedScalars kind text))

/-- The generated `len_max` refines the exact `LEN_MAX`. -/
def lenMaxRefines : Prop :=
  ∀ capacity : Std.Usize,
    ResultRefinesUsize (exact.len_max capacity) (Stc.Exact.lenMax capacity.val)

/-- The generated `len_code_unit` refines the exact `LEN_CODE_UNIT`. The
`USTRING` case returns the UTF-8 byte length, which the TCB byte-length
invariant relates to the decoded scalars. -/
def lenCodeUnitRefines : Prop :=
  ∀ (kind : exact.StringKind) (text : Str),
    (kind = exact.StringKind.Sbcs → isAsciiBytes text.val = true) →
      ResultRefinesUsize (exact.len_code_unit kind text)
        (Stc.Exact.lenCodeUnit (decodeStringKind kind) (decodedScalars kind text))

end Stc.Bridge.Exact
