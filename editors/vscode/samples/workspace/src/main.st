(* Sample workspace for the stc VS Code extension.

   This project is the local fixture for language-server navigation and for a
   launch debug session. It is self-contained and uses no absolute paths. *)

VAR_GLOBAL
  (* The debug test asserts these initial values. *)
  INCREMENT : DINT := 1;
  TOTAL : DINT := 0;
END_VAR

FUNCTION AddStep : DINT
VAR_INPUT
  value : DINT;
END_VAR
VAR
  incremented : DINT;
END_VAR
  incremented := INCREMENT;
  AddStep := value + incremented;
END_FUNCTION

FUNCTION_BLOCK Accumulator
VAR_INPUT
  increment : DINT := 1;
END_VAR
VAR_OUTPUT
  total : DINT;
END_VAR
  total := total + increment;
END_FUNCTION_BLOCK

PROGRAM Main
VAR
  accumulator : Accumulator;
  sum : DINT;
  cycles : DINT;
END_VAR
  sum := AddStep(sum);
  accumulator(increment := INCREMENT, total => sum);
  cycles := cycles + 1;
  TOTAL := sum;
END_PROGRAM

CONFIGURATION Sample
RESOURCE Cpu ON Cpu
  TASK MainTask(INTERVAL := T#10ms, PRIORITY := 1);
  PROGRAM sample WITH MainTask : Main;
END_RESOURCE
END_CONFIGURATION
