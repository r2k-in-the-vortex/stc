VAR_GLOBAL
  A : DINT;
  B : DINT;
  C : DINT;
  D : DINT;
  E : DINT;
  F : DINT;
  G : DINT;
  H : DINT;
END_VAR
PROGRAM Main
  A := ABS(-7);
  B := SEL(FALSE, 10, 20);
  C := SEL(TRUE, 10, 20);
  D := MIN(10, 4);
  E := MAX(10, 4);
  F := LIMIT(0, 15, 10);
  G := MUX(1, 11, 22, 33);
  H := MUX(5, 11, 22, 33);
END_PROGRAM
