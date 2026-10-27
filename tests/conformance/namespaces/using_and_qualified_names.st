NAMESPACE Lib
FUNCTION Twice : DINT
VAR_INPUT
  a : DINT;
END_VAR
  Twice := 2 * a;
END_FUNCTION
END_NAMESPACE
PROGRAM Main
USING Lib;
VAR
  x : DINT;
  zero : DINT;
END_VAR
  x := Twice(3);
  IF x <> 6 THEN
    zero := 0;
    x := 1 / zero;
  END_IF;
  x := Lib.Twice(4);
  IF x <> 8 THEN
    zero := 0;
    x := 1 / zero;
  END_IF;
END_PROGRAM
