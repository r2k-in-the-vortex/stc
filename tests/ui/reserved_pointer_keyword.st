// STC-LEX-059: POINTER, ADR, and SIZEOF are reserved in every context.
PROGRAM P
VAR
  POINTER : DINT; //~ ERROR STC-E-GRAM-105
  x : DINT;
END_VAR
  x := 1;
END_PROGRAM
