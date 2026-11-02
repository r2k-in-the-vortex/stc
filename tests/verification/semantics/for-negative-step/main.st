VAR_GLOBAL
  R : DINT;
  Hits : DINT;
END_VAR
PROGRAM Main
VAR
  i : DINT;
END_VAR
  FOR i := 3 TO 1 BY -1 DO
    R := R + i;
    Hits := Hits + 1;
  END_FOR;
END_PROGRAM
