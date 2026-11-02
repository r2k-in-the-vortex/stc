VAR_GLOBAL
  A : DINT;
  B : BOOL;
  C : WORD;
  D : BOOL;
END_VAR
PROGRAM Main
  A := -5;
  A := +A;
  B := NOT B;
  C := NOT WORD#16#00F0;
  D := NOT (A = -5);
END_PROGRAM
