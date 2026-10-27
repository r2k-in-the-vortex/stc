VAR_GLOBAL
  observed : DINT;
END_VAR
FUNCTION AddOne : DINT
VAR_INPUT
  a : DINT;
END_VAR
  AddOne := a + 1;
END_FUNCTION
FUNCTION_BLOCK Counter
VAR_INPUT
  delta : DINT;
END_VAR
VAR_OUTPUT
  value : DINT;
END_VAR
  value := value + delta;
  observed := value;
END_FUNCTION_BLOCK
PROGRAM Main
VAR
  c : Counter;
  x : DINT;
  zero : DINT;
END_VAR
  c(delta := 2);
  x := AddOne(observed);
  IF x <> 3 THEN
    zero := 0;
    x := 1 / zero;
  END_IF;
END_PROGRAM
