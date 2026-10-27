PROGRAM Main
VAR
  input AT %IX0.0 : BOOL;
  output AT %QX0.0 : BOOL;
  zero : DINT;
END_VAR
  IF input THEN
    zero := 0;
    output := 1 / zero > 0;
  END_IF;
  output := NOT input;
END_PROGRAM
