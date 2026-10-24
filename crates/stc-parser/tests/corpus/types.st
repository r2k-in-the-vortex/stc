TYPE
  Speed : INT (0..100);
  Color : (Red, Green := 2, Blue);
  Flags : DWORD (Read := 16#01, Write := 16#02);
  Vector : ARRAY [1..3] OF REAL;
  Matrix : ARRAY [0..1, 0..1] OF INT;
  Point : STRUCT OVERLAP
    x : REAL;
    y : REAL;
  END_STRUCT;
  Name : STRING[32];
  Ptr : REF_TO Point;
END_TYPE
