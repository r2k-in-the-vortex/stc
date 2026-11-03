/-
Shared string model for the widened bridge.

This module is compiled only in the non-default `lean/aeneas` package. It
models the string content that the widened generated operations observe:
Aeneas represents `str` as a byte slice (`Str := Slice U8`), while the exact
model in `Stc.Exact.Strings` counts characters over a scalar list.

The scalar sequence itself is a TCB assumption (`strScalars`): the generated
string functions are opaque, and Lean has no definition of the compiler's
UTF-8 table. The assumption is paired with the byte-length invariant in
`WideExternals`, where every generated string axiom is recorded with the
equation it must satisfy.
-/

import StcGenerated.Exact.Model
import Stc.Exact.Strings

namespace Stc.Bridge.Exact

open Aeneas Aeneas.Std

noncomputable section

/-- Decode a generated string kind. -/
def decodeStringKind : exact.StringKind → Stc.Exact.StringKind
  | .Sbcs => .sbcs
  | .Dbcs => .dbcs
  | .Unicode => .unicode

/-- The byte predicate `str::is_ascii` tests: every byte is below `0x80`. -/
def isAsciiBytes (bytes : List Std.U8) : Bool :=
  bytes.all (fun byte => decide (byte.val < 0x80))

/-- TCB model of the decoded string contents: the scalar sequence the generated
string operations observe. The compiler guarantees valid UTF-8 for `str` data. -/
axiom strScalars : Str → List Nat

/-- The scalar list a string kind observes. `SBCS` observes one scalar per
stored byte; `DBCS` and `USTRING` observe the UTF-8 decoding. -/
def decodedScalars : exact.StringKind → Str → List Nat
  | .Sbcs => fun text => text.val.map (fun byte => byte.val)
  | .Dbcs => strScalars
  | .Unicode => strScalars

end

end Stc.Bridge.Exact
