VAR_GLOBAL
  WSum : DINT;
  RSum : DINT;
  Count : DINT;
END_VAR
PROGRAM Main
VAR
  i : DINT;
END_VAR
  i := 0;
  WHILE i < 5 DO
    i := i + 1;
    IF i = 2 THEN
      CONTINUE;
    END_IF;
    IF i = 4 THEN
      EXIT;
    END_IF;
    WSum := WSum + i;
  END_WHILE;
  REPEAT
    Count := Count + 1;
    IF Count = 1 THEN
      CONTINUE;
    END_IF;
    RSum := RSum + Count;
  UNTIL Count >= 3 END_REPEAT;
END_PROGRAM
