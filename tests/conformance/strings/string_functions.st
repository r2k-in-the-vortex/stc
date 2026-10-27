PROGRAM Main
VAR
  a : STRING[8];
  b : STRING[8];
  s : STRING[16];
  t : STRING[16];
  n : DINT;
  zero : DINT;
END_VAR
  a := 'ab';
  b := 'cd';
  s := CONCAT(a, b);
  IF s <> 'abcd' THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  IF LEN(s) <> 4 THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  IF FIND(s, b) <> 3 THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  t := LEFT(s, 2);
  IF t <> 'ab' THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  t := MID(s, 2, 2);
  IF t <> 'bc' THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  t := RIGHT(s, 2);
  IF t <> 'cd' THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
END_PROGRAM
