(* Pure tests for the mqtt example. The network and process image are not
   involved; `stc test` compiles these {stc:test} POUs into separate artifacts. *)

{stc:test}
FUNCTION ClampSpeedKeepsTheRange : BOOL
  STC_ASSERT(ClampSpeed(-5.0) = 0.0, "negative request clamps to zero");
  STC_ASSERT(ClampSpeed(42.5) = 42.5, "in-range request is preserved");
  STC_ASSERT(ClampSpeed(150.0) = 100.0, "request above the range clamps to 100");
  ClampSpeedKeepsTheRange := TRUE;
END_FUNCTION
