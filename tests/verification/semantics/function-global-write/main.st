VAR_GLOBAL
  Count : DINT;
  R : DINT;
END_VAR
FUNCTION BumpAndGet : DINT
VAR_INPUT
  x : DINT;
END_VAR
  Count := Count + 1;
  BumpAndGet := x + 1;
END_FUNCTION

PROGRAM Main
  R := BumpAndGet(5);
  R := R + BumpAndGet(R);
END_PROGRAM
