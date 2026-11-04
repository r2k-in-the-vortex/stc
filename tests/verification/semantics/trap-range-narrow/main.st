VAR_GLOBAL
  Before : DINT;
END_VAR
PROGRAM Main
VAR
  x : DINT;
  y : SINT;
END_VAR
  x := 300;
  Before := 1;
  y := DINT_TO_SINT(x);
END_PROGRAM
