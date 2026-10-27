(* Nested (* block comments *) are legal, and comments preserve UTF-8 text: hé. *)
// A line comment ends at the line feed.
PROGRAM Main
VAR
  dec : DINT := 1_000;
  hex : DINT := 16#FF;
  bin : DINT := 2#1010;
  realv : REAL := 1.5e2;
  dur : TIME := T#1s500ms;
  todv : TOD := TOD#12:30:00;
  datev : DATE := D#2026-10-27;
  s : STRING := 'a$Lb$T';
  w : WSTRING := "wide";
  b : BOOL := BOOL#TRUE;
  c : CHAR := CHAR#'x';
END_VAR
  dec := dec + hex + bin;
END_PROGRAM
