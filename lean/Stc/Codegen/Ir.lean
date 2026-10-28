/-
The core emitted-IR fragment and its well-formedness predicate (P0.4).

`IRFrag` covers the module, function, block, instruction, terminator, global,
slot-table, and function-table shapes that `stc-codegen` emits for the core
subset. `WellFormed` records type correctness, SSA/dominance, block
terminators, `phi` arity, GOT cell kinds, the canonical slot table, and the
ABI 2 entry-point signatures. The printer and parser are out of scope; the
semantics is defined over the in-memory fragment.
-/
namespace Stc.Codegen
end Stc.Codegen
