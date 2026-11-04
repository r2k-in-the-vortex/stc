(* Smoke tests for the sample workspace. *)

{stc:test}
FUNCTION AddStepWorks : BOOL
VAR
  result : DINT;
END_VAR
  result := AddStep(2);
  STC_ASSERT(result = 3, "AddStep(2) should be 3");
  AddStepWorks := TRUE;
END_FUNCTION

{stc:test}
FUNCTION AccumulatorWorks : BOOL
VAR
  accumulator : Accumulator;
  value : DINT;
END_VAR
  accumulator(increment := 2, total => value);
  accumulator(increment := 3, total => value);
  STC_ASSERT(value = 5, "the accumulator should add each step");
  AccumulatorWorks := TRUE;
END_FUNCTION
