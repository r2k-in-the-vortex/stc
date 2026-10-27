PROGRAM Main
USING dep;
VAR
  x : DINT;
  zero : DINT;
END_VAR
  x := Twice(3);
  IF x <> 6 THEN
    zero := 0;
    x := 1 / zero;
  END_IF;
END_PROGRAM
