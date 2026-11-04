VAR_GLOBAL
  Count : DINT;
END_VAR
FUNCTION Boom : DINT
VAR_INPUT
  z : DINT;
END_VAR
  Count := Count + 1;
  Boom := 1 / z;
END_FUNCTION

PROGRAM Main
  Count := 5;
  Boom(0);
END_PROGRAM
