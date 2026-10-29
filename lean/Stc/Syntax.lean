/-
Surface syntax for the Phase 0 core subset (P0.1).

This module is the umbrella for the token model, the lossless CST contract, and
the surface AST. P0.0 fixes the module layout so later sub-phases add
declarations without moving imports.

Scope is frozen by `specification/Verification/Semantics-Charter.md` §6 and
`specification/Verification/Plan-Phase0.md` §3.3.
-/
import Stc.Syntax.Tokens
import Stc.Syntax.Cst
import Stc.Syntax.Ast

namespace Stc.Syntax
end Stc.Syntax
