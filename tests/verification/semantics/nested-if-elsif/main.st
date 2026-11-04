VAR_GLOBAL
  R : DINT;
END_VAR
PROGRAM Main
VAR
  i : DINT;
END_VAR
  i := 2;
  IF i = 1 THEN
    R := 10;
  ELSIF i = 2 THEN
    R := 20;
  ELSE
    R := 30;
  END_IF;
END_PROGRAM
