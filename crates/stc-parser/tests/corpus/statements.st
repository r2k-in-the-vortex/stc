PROGRAM Main
VAR
  a, b : DINT;
  flag : BOOL;
  p : REF_TO DINT;
  arr : ARRAY [0..9] OF DINT;
  fb : FB;
END_VAR
  a := 1 + 2 * 3 ** 2;
  b := -2 ** 2;
  flag := NOT a = b;
  p := REF(a);
  p := NULL;
  p^ := b;
  p ?= REF(b);
  fb(in := a, out => b, NOT flag => a);
  ;
  IF flag THEN
    a := a + 1;
  ELSIF b > 0 THEN
    b := b - 1;
  ELSE
    a := 0;
  END_IF;
  CASE a OF
    0 : b := 1;
    1, 2 : b := 2;
    3..5 : b := 3;
    ELSE
      b := 4;
  END_CASE;
  FOR a := 0 TO 9 BY 1 DO
    arr[a] := a;
  END_FOR;
  WHILE a < 10 DO
    a := a + 1;
  END_WHILE;
  REPEAT
    a := a - 1;
  UNTIL a = 0
  END_REPEAT;
  EXIT;
  CONTINUE;
  RETURN;
END_PROGRAM
