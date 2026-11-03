/-
Trust-boundary equations for the string operations added by the widened
generated model.

This module is compiled only in the non-default `lean/aeneas` package. Each
proposition states what one opaque generated operation must compute, in terms
of the shared string model in `Bridge.Exact.StrModel` and the exact string
model in `Stc.Exact.Strings`. They are TCB assumptions, not theorems: the
generated functions have no Lean body, so the equations cannot be proved
inside Lean. Their exact-side models and the byte-length invariant are
recorded in `Audit.lean`.
-/

import Bridge.Exact.StrModel
import Bridge.Exact.Refinement
import Stc.Exact.Strings

namespace Stc.Bridge.Exact

open Aeneas Aeneas.Std

/-- TCB assumption: the decoded scalar sequence encodes to the stored byte
length. This is the UTF-8 validity invariant the compiler maintains for `str`
data. -/
axiom strScalars_utf8ByteLength (text : Str) :
    Stc.Exact.utf8ByteLength (strScalars text) = text.val.length

/-- TCB assumption: `str::len` returns the byte length. -/
axiom strLen_refines (text : Str) :
    ResultRefinesUsize (exact.core.str.Str.len text) text.val.length

/-- TCB assumption: `str::is_ascii` tests every stored byte against `0x80`. -/
axiom strIsAscii_refines (text : Str) :
    ResultRefinesBool (exact.core.str.Str.is_ascii text) (isAsciiBytes text.val)

/-- TCB assumption: `str::chars` followed by `Iterator::count` counts the
decoded scalar sequence. -/
axiom strCharsCount_refines (text : Str) :
    ResultRefinesUsize
      (do let chars ← core.str.Str.chars text
          exact.core.str.iter.Chars.Insts.CoreIterTraitsIteratorIteratorChar.count chars)
      (strScalars text).length

/-- TCB assumption: `str::encode_utf16` followed by `Iterator::count` counts
the UTF-16 code units of the decoded scalar sequence. -/
axiom strEncodeUtf16Count_refines (text : Str) :
    ResultRefinesUsize
      (do let units ← exact.core.str.Str.encode_utf16 text
          exact.core.str.iter.EncodeUtf16.Insts.CoreIterTraitsIteratorIteratorU16.count units)
      (Stc.Exact.utf16Length (strScalars text))

end Stc.Bridge.Exact
