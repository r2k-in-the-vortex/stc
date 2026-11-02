VAR_GLOBAL
  R : DINT;
END_VAR
PROGRAM Main
VAR
  i : SINT;
END_VAR
  i := SINT#-1;
  CASE i OF
    -1: R := 1;
    0: R := 2;
  ELSE
    R := 3;
  END_CASE;
END_PROGRAM
