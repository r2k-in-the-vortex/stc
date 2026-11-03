VAR_GLOBAL
  Source : DINT;
  Mirror : DINT;
END_VAR

FUNCTION ReadSource : DINT
  ReadSource := Source * 2;
END_FUNCTION

PROGRAM Main
  Source := 21;
  Mirror := ReadSource();
END_PROGRAM
