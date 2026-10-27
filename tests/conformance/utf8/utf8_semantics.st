PROGRAM Main
VAR
  s : USTRING[8];
  t : USTRING[8];
  u : USTRING[8];
  c : UCHAR;
  n : DINT;
  m : DINT;
  zero : DINT;
END_VAR
  s := U#'hé🙂';
  n := LEN(s);
  IF n <> 3 THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  m := LEN_CODE_UNIT(s);
  IF m <> 7 THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  IF LEN_MAX(s) <> 8 THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  c := s[1];
  IF c <> UCHAR#'h' THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  c := s[2];
  IF c <> UCHAR#${E9} THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  t := U#'ab';
  u := U#'cd';
  t := CONCAT(t, u);
  IF LEN(t) <> 4 THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  IF FIND(t, U#'cd') <> 3 THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  t := LEFT(s, 2);
  IF LEN(t) <> 2 THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  t := MID(s, 1, 2);
  IF t <> U#'é' THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  t := RIGHT(s, 1);
  IF t <> U#'🙂' THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
END_PROGRAM
