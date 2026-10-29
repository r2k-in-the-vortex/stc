(* Trapping online-change candidate: same roots and layout as the base for
   the main task, plus a no-interval trap task. A task without INTERVAL is
   due on every scan, so the trap fires on the first scan after activation
   and the host must roll back within its one-cycle rollback window. *)

VAR_GLOBAL CONSTANT
  WORK : DINT := 60000;
  INCREMENT : DINT := 3;
END_VAR

VAR_GLOBAL
  Spare : DINT;
  Progress : DINT;
END_VAR

FUNCTION_BLOCK Accumulator
VAR_INPUT
  Amount : DINT := 1;
END_VAR
VAR_OUTPUT PUBLIC
  Value : DINT;
END_VAR
  Value := Value + Amount;
END_FUNCTION_BLOCK

PROGRAM Main
VAR
  acc : Accumulator;
  sum : DINT;
  i : DINT;
END_VAR
  sum := 0;
  FOR i := 1 TO WORK DO
    sum := sum + (i MOD 97) * INCREMENT;
  END_FOR;
  acc(Amount := INCREMENT);
  Progress := acc.Value + sum;
END_PROGRAM

PROGRAM TrapProg
VAR
  zero : DINT;
  value : DINT;
END_VAR
  zero := 0;
  value := 1 / zero;
END_PROGRAM

CONFIGURATION RtBench
RESOURCE Cpu ON Cpu
  TASK MainTask(INTERVAL := T#1ms, PRIORITY := 1);
  TASK TrapTask(PRIORITY := 2);
  PROGRAM main WITH MainTask : Main;
  PROGRAM trap_inst WITH TrapTask : TrapProg;
END_RESOURCE
END_CONFIGURATION
