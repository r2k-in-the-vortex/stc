PROGRAM Main
VAR RETAIN
  counter : DINT := 7;
END_VAR
VAR
  zero : DINT;
END_VAR
  IF counter = 7 THEN
    counter := 42;
  ELSIF counter = 42 THEN
    counter := 43;
  ELSE
    zero := 0;
    counter := 1 / zero;
  END_IF;
END_PROGRAM
