(* online-change: a small program for the hot-swap walkthrough.

   `stc run --watch` builds against the active runtime baseline and deploys
   each successful change at a cycle boundary. Editing `INCREMENT` or adding a
   field is a compatible change; changing a type or layout is rejected and
   requires the documented cold download. See README.md.

   `stc` is not a safety PLC and is not responsible for safety. It can control
   real machinery, but safety must be provided by a separate safety system and
   machine design as required by law and regulation. *)

VAR_GLOBAL CONSTANT
  INCREMENT : DINT := 1;
END_VAR

FUNCTION_BLOCK Counter
VAR PUBLIC
  Ticks : DINT;
END_VAR
VAR_OUTPUT PUBLIC
  Count : DINT;
  Total : DINT;
END_VAR
VAR RETAIN PUBLIC
  Retained : DINT;
END_VAR
  Ticks := Ticks + 1;
  Count := Count + INCREMENT;
  Total := Total + INCREMENT;
  Retained := Retained + INCREMENT;
END_FUNCTION_BLOCK

PROGRAM Main
VAR
  counter : Counter;
  count : DINT;
  ticks : DINT;
END_VAR
  counter();
  count := counter.Count;
  ticks := counter.Ticks;
END_PROGRAM

CONFIGURATION OnlineDemo
RESOURCE Cpu ON Cpu
  TASK MainTask(INTERVAL := T#10ms, PRIORITY := 1);
  PROGRAM main WITH MainTask : Main;
END_RESOURCE
END_CONFIGURATION
