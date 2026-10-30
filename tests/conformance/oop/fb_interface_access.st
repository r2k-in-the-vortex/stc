FUNCTION_BLOCK Counter
VAR_INPUT
  inc : DINT;
END_VAR
VAR_OUTPUT
  q : DINT;
END_VAR
VAR
  cv : DINT;
END_VAR
  cv := cv + inc;
  q := cv;
END_FUNCTION_BLOCK

PROGRAM Main
VAR
  c : Counter;
  x : DINT;
END_VAR
  (* Default FB interface fields are externally accessible through both the
     call form and direct member access. *)
  c(inc := 1, q => x);
  c.inc := 2;
  c();
  x := c.q;
END_PROGRAM
