VAR_GLOBAL
  Total : DINT;
  Chosen : DINT;
END_VAR

PROGRAM Main
VAR
  i : DINT;
END_VAR
  FOR i := 1 TO 4 DO
    IF i > 2 THEN
      Total := Total + i;
    END_IF;
  END_FOR;
  Chosen := Total;
END_PROGRAM
