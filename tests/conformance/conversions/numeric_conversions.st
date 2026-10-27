PROGRAM Main
VAR
  i : INT := -5;
  d : DINT;
  r : REAL;
  b : BOOL;
  u : UINT;
  zero : DINT;
END_VAR
  d := INT_TO_DINT(i);
  IF d <> -5 THEN
    zero := 0;
    d := 1 / zero;
  END_IF;
  r := INT_TO_REAL(i);
  IF r > -4.9 OR r < -5.1 THEN
    zero := 0;
    d := 1 / zero;
  END_IF;
  b := INT_TO_BOOL(i);
  IF NOT b THEN
    zero := 0;
    d := 1 / zero;
  END_IF;
  u := INT_TO_UINT(5);
  IF u <> 5 THEN
    zero := 0;
    d := 1 / zero;
  END_IF;
END_PROGRAM
