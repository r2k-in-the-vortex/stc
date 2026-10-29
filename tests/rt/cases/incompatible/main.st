(* Incompatible candidate: changes the type of Spare. The runtime must
   reject the online change or require a cold download; the old generation
   must keep running. *)

VAR_GLOBAL CONSTANT
  WORK : DINT := 60000;
  INCREMENT : DINT := 3;
END_VAR

VAR_GLOBAL
  Spare : LREAL;
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

CONFIGURATION RtBench
RESOURCE Cpu ON Cpu
  TASK MainTask(INTERVAL := T#1ms, PRIORITY := 1);
  PROGRAM main WITH MainTask : Main;
END_RESOURCE
END_CONFIGURATION
