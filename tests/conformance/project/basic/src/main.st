PROGRAM Main
VAR
  counter : DINT;
  zero : DINT;
END_VAR
  counter := counter + 1;
  IF counter <> 1 THEN
    zero := 0;
    counter := 1 / zero;
  END_IF;
END_PROGRAM
