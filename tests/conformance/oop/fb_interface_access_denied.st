FUNCTION_BLOCK Counter
VAR_INPUT PRIVATE
  inc : DINT;
END_VAR
VAR_OUTPUT PROTECTED
  q : DINT;
END_VAR
END_FUNCTION_BLOCK

PROGRAM Main
VAR
  c : Counter;
  x : DINT;
END_VAR
  (* An explicit qualifier overrides the interface default in both forms. *)
  c(inc := 1, q => x);
  c.inc := 2;
  x := c.q;
END_PROGRAM
