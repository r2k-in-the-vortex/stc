/-
The lossless CST contract for the core surface language (P0.1).

The contract states that token and trivia spans cover the source exactly, in
order, with no gaps or overlaps. The parser proofs in Phase 4 consume the
contract rather than a literal copy of the rowan tree.
-/
namespace Stc.Syntax
end Stc.Syntax
