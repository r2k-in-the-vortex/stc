TYPE
  Color : (Red, Green := 5, Blue);
  Small : INT(0..10);
  Point : STRUCT
    x : DINT;
    y : DINT;
  END_STRUCT;
END_TYPE
PROGRAM Main
VAR
  c : Color;
  s : Small;
  a : ARRAY[1..3] OF DINT;
  p : Point := (x := 1, y := 2);
  q : ARRAY[1..2] OF DINT := [10, 20];
  n : DINT;
END_VAR
  c := Green;
  s := 5;
  a[1] := 3;
  p.x := p.y;
  q[2] := n;
  CASE c OF
    Red: n := 1;
    Green: n := 2;
  ELSE
    n := 0;
  END_CASE;
END_PROGRAM
