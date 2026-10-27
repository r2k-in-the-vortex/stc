PROGRAM Main
VAR
  c : CHAR;
  w : WCHAR;
  z : UCHAR;
  n : DINT;
  zero : DINT;
END_VAR
  c := CHAR#'A';
  z := CHAR_TO_UCHAR(c);
  IF z <> UCHAR#'A' THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  c := UCHAR_TO_CHAR(z);
  IF c <> CHAR#'A' THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  w := WCHAR#'B';
  z := WCHAR_TO_UCHAR(w);
  IF z <> UCHAR#'B' THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  w := UCHAR_TO_WCHAR(z);
  IF w <> WCHAR#'B' THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
END_PROGRAM
