VAR_GLOBAL
  R : DINT;
END_VAR
PROGRAM Main
VAR
  i : DINT;
END_VAR
  i := 3;
  CASE i OF
    1: R := 10;
    2..4: R := 20;
    5: R := 30;
  ELSE
    R := 99;
  END_CASE;
END_PROGRAM
