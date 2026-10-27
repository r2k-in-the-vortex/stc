PROGRAM Main
VAR
  i : DINT;
  sum : DINT;
  n : DINT;
  zero : DINT;
  flag : BOOL;
END_VAR
  sum := 0;
  FOR i := 1 TO 5 DO
    sum := sum + i;
  END_FOR;
  IF sum <> 15 THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  n := 0;
  WHILE n < 3 DO
    n := n + 1;
  END_WHILE;
  IF n <> 3 THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  REPEAT
    n := n - 1;
  UNTIL n = 0
  END_REPEAT;
  IF n <> 0 THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  CASE sum OF
    15: flag := TRUE;
  ELSE
    flag := FALSE;
  END_CASE;
  IF NOT flag THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
END_PROGRAM
