FUNCTION IsEven : BOOL
VAR_INPUT
  n : DINT;
END_VAR
  IF n = 0 THEN
    IsEven := TRUE;
  ELSE
    IsEven := IsOdd(n - 1);
  END_IF;
END_FUNCTION

FUNCTION IsOdd : BOOL
VAR_INPUT
  n : DINT;
END_VAR
  IF n = 0 THEN
    IsOdd := FALSE;
  ELSE
    IsOdd := IsEven(n - 1);
  END_IF;
END_FUNCTION

VAR_GLOBAL
  R : BOOL;
END_VAR
PROGRAM Main
  R := IsEven(10);
END_PROGRAM
