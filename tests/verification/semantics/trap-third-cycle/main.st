VAR_GLOBAL
  Count : DINT;
END_VAR
PROGRAM Main
VAR
  z : DINT;
END_VAR
  Count := Count + 1;
  IF Count = 3 THEN
    z := 0;
    Count := 1 / z;
  END_IF;
END_PROGRAM
