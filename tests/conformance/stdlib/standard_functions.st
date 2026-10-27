PROGRAM Main
VAR
  a : DINT;
  b : DINT;
  c : DINT;
  r : REAL;
  w : WORD;
  zero : DINT;
END_VAR
  a := ABS(-3);
  IF a <> 3 THEN
    zero := 0;
    a := 1 / zero;
  END_IF;
  a := MIN(3, 5);
  IF a <> 3 THEN
    zero := 0;
    a := 1 / zero;
  END_IF;
  a := MAX(3, 5);
  IF a <> 5 THEN
    zero := 0;
    a := 1 / zero;
  END_IF;
  a := LIMIT(0, 10, 4);
  IF a <> 4 THEN
    zero := 0;
    a := 1 / zero;
  END_IF;
  a := SEL(FALSE, 1, 2);
  IF a <> 1 THEN
    zero := 0;
    a := 1 / zero;
  END_IF;
  a := MUX(1, 10, 20, 30);
  IF a <> 20 THEN
    zero := 0;
    a := 1 / zero;
  END_IF;
  r := SQRT(9.0);
  IF r < 2.9 OR r > 3.1 THEN
    zero := 0;
    a := 1 / zero;
  END_IF;
  r := EXPT(2.0, 3.0);
  IF r < 7.9 OR r > 8.1 THEN
    zero := 0;
    a := 1 / zero;
  END_IF;
  w := SHL(WORD#1, 3);
  IF w <> WORD#8 THEN
    zero := 0;
    a := 1 / zero;
  END_IF;
  w := ROR(WORD#1, 1);
  IF w <> WORD#32768 THEN
    zero := 0;
    a := 1 / zero;
  END_IF;
  b := ADD(2, 3);
  c := MUL(4, 5);
  IF b <> 5 OR c <> 20 THEN
    zero := 0;
    a := 1 / zero;
  END_IF;
END_PROGRAM
