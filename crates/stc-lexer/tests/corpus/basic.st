// Basic declaration and statement snapshot.
{stc:test}
PROGRAM Main
VAR
  counter : DINT := 0;
  flag : BOOL;
END_VAR
  (* update the counter *)
  counter := counter + 1; // each scan
  flag := counter MOD 2 = 0;
  IF flag THEN
    counter := counter - 1;
  ELSE
    counter := counter + 2;
  END_IF
END_PROGRAM
