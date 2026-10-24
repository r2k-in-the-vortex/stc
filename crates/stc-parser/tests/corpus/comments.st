// Leading line comment
(* leading block comment *)
{stc:test}
FUNCTION Commented : INT
VAR
  x : INT; // trailing line comment
  (* between declarations *)
  y : INT;
END_VAR
  // before a statement
  x := y + 1; (* trailing block *)
  Commented := x;
END_FUNCTION
// trailing comment at end of file
