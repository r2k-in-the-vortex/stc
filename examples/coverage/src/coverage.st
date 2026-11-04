(* coverage-demo: production functions with known covered and uncovered
   statements, plus a test-only POU that lives in a production file.

   `AddValues` is called by a test, `Average` is never called, and `Classify` is
   called with a positive value so its IF and THEN body run while its ELSE
   body does not. `ExcludedHelper` is marked `{stc:test}`: it is compiled only
   into test artifacts and is excluded from the coverage report even though it
   sits next to measurable production code. *)

FUNCTION AddValues : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  AddValues := a + b;
END_FUNCTION

FUNCTION Average : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  Average := (a + b) / 2;
END_FUNCTION

FUNCTION Classify : DINT
VAR_INPUT
  value : DINT;
END_VAR
  IF value >= 0 THEN
    Classify := 1;
  ELSE
    Classify := -1;
  END_IF;
END_FUNCTION

(* A test-only POU in a production source file. `stc test` discovers and runs
   it, but its statements are test code and never enter the coverage report. *)
{stc:test}
FUNCTION ExcludedHelper : BOOL
VAR
  doubled : DINT;
END_VAR
  doubled := 21 * 2;
  ExcludedHelper := doubled = 42;
END_FUNCTION
