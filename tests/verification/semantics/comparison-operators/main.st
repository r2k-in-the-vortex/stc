VAR_GLOBAL
  E : BOOL;
  NE : BOOL;
  LT : BOOL;
  LE : BOOL;
  GT : BOOL;
  GE : BOOL;
END_VAR
PROGRAM Main
  E := 3 = 3;
  NE := 3 <> 4;
  LT := 3 < 4;
  LE := 4 <= 4;
  GT := 5 > 4;
  GE := 4 >= 5;
END_PROGRAM
