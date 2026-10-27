PROGRAM Main
VAR
  x : DINT;
  y : DINT;
  zero : DINT;
  flag : BOOL;
  a : DINT;
  b : DINT;
END_VAR
  x := 2 + 3 * 4;
  IF x <> 14 THEN
    zero := 0;
    x := 1 / zero;
  END_IF;
  y := 2 ** 3 ** 2;
  IF y <> 64 THEN
    zero := 0;
    x := 1 / zero;
  END_IF;
  y := -2 ** 2;
  IF y <> 4 THEN
    zero := 0;
    x := 1 / zero;
  END_IF;
  flag := NOT TRUE = FALSE;
  IF NOT flag THEN
    zero := 0;
    x := 1 / zero;
  END_IF;
  a := 1;
  b := 4;
  flag := a + 2 < b = TRUE;
  IF NOT flag THEN
    zero := 0;
    x := 1 / zero;
  END_IF;
  a := 0;
  flag := (a <> 0) AND (10 / a > 1);
  IF flag THEN
    zero := 0;
    x := 1 / zero;
  END_IF;
END_PROGRAM
