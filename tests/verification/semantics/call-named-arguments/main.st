FUNCTION Difference : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  Difference := a - b;
END_FUNCTION

VAR_GLOBAL
  R : DINT;
END_VAR
PROGRAM Main
  R := Difference(b := 3, a := 10);
END_PROGRAM
