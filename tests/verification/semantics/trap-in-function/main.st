FUNCTION Ratio : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  Ratio := a / b;
END_FUNCTION

VAR_GLOBAL
  Before : DINT;
END_VAR
PROGRAM Main
  Before := 1;
  Before := Ratio(10, 0);
END_PROGRAM
