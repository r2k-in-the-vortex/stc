/-
The source dynamic semantics for the core subset (P0.3).

`Step` is the small-step relation over configurations with explicit call and
control stacks; `stcSem` is the fuel-based recursive reference model. The
source soundness theorems (`eval_deterministic`, `progress`, `preservation`,
`type_soundness`, `trap_soundness`, `checked_arith_traps_exactly`) and the
recursive-model equivalence are proved here or in the interpreter module.
-/
namespace Stc.Sem
end Stc.Sem
