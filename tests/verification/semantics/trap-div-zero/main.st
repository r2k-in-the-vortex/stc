VAR_GLOBAL
  Before : DINT;
END_VAR

PROGRAM Main
VAR
  x : DINT;
END_VAR
  Before := 7;
  x := Before / 0;
END_PROGRAM
