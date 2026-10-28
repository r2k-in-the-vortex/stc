(* Unit tests live in the [test] source set. `stc test` builds each
   {stc:test} POU into its own artifact and runs it in a separate process. *)

{stc:test}
FUNCTION AddTwoWorks : BOOL
VAR
  result : DINT;
END_VAR
  result := AddTwo(2, 3);
  STC_ASSERT(result = 5, "AddTwo(2, 3) should be 5");
  AddTwoWorks := TRUE;
END_FUNCTION

{stc:test}
FUNCTION CounterCounts : BOOL
VAR
  counter : Counter;
  v : DINT;
END_VAR
  counter(increment := 2, value => v);
  STC_ASSERT(v = 2, "the first call counts one increment");
  counter(increment := 2, value => v);
  STC_ASSERT(v = 4, "the counter keeps its state");
  CounterCounts := TRUE;
END_FUNCTION

{stc:test}
FUNCTION ControlFlowWorks : BOOL
VAR
  i : DINT;
  sum : DINT;
  kind : DINT;
END_VAR
  FOR i := 1 TO 3 DO
    sum := sum + i;
  END_FOR;
  STC_ASSERT(sum = 6, "1 + 2 + 3 should be 6");

  CASE sum OF
  6:
    kind := 1;
  7:
    kind := 2;
  ELSE
    kind := 3;
  END_CASE;
  STC_ASSERT(kind = 1, "CASE should select the 6 arm");

  WHILE sum > 0 DO
    sum := sum - 2;
  END_WHILE;
  STC_ASSERT(sum = 0, "WHILE should count down to zero");

  REPEAT
    sum := sum + 3;
  UNTIL sum >= 12
  END_REPEAT;
  STC_ASSERT(sum = 12, "REPEAT should run until the condition holds");
  ControlFlowWorks := TRUE;
END_FUNCTION
