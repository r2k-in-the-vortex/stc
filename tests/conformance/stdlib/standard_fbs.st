PROGRAM Main
VAR
  cu : CTU;
  rt : R_TRIG;
  flag : BOOL;
  n : DINT;
  zero : DINT;
END_VAR
  cu(CU := TRUE, R := FALSE, PV := 3);
  IF cu.CV <> 1 THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  rt(CLK := flag);
  IF rt.Q THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
  flag := TRUE;
  rt(CLK := flag);
  IF NOT rt.Q THEN
    zero := 0;
    n := 1 / zero;
  END_IF;
END_PROGRAM
