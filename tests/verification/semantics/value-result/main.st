FUNCTION Add : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  Add := a + b;
END_FUNCTION

VAR_GLOBAL
  Result : DINT;
END_VAR

PROGRAM Main
VAR
  x : DINT;
END_VAR
  x := Add(40, 2);
  Result := x;
END_PROGRAM
