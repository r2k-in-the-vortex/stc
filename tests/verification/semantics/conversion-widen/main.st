VAR_GLOBAL
  A : DINT;
  B : DINT;
  C : UDINT;
  D : DINT;
END_VAR
PROGRAM Main
VAR
  s : SINT;
  u : USINT;
  w : WORD;
END_VAR
  s := SINT#-5;
  A := SINT_TO_DINT(s);
  u := USINT#200;
  B := USINT_TO_DINT(u);
  C := SINT_TO_UDINT(s);
  w := WORD#16#FFFE;
  D := WORD_TO_DINT(w);
END_PROGRAM
