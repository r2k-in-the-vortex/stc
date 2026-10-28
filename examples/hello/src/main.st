(* Hello: a small IEC 61131-3 Structured Text project.
   It exercises scalar variables, operators, control flow, a function,
   a function block, the standard TON timer, and a task configuration. *)

VAR_GLOBAL CONSTANT
  INCREMENT : DINT := 1;
END_VAR

FUNCTION AddTwo : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  AddTwo := a + b;
END_FUNCTION

FUNCTION_BLOCK Counter
VAR_INPUT
  increment : DINT := 1;
END_VAR
VAR_OUTPUT
  value : DINT;
END_VAR
  value := value + increment;
END_FUNCTION_BLOCK

PROGRAM Main
VAR
  counter : Counter;
  sum : DINT;
  blink : TON;
  lamp : BOOL;
  i : DINT;
END_VAR
  counter(increment := INCREMENT, value => sum);
  sum := AddTwo(sum, INCREMENT);

  (* FOR and IF are ordinary ST control flow. *)
  FOR i := 1 TO 3 DO
    sum := sum + i;
  END_FOR;
  IF sum > 10 THEN
    sum := sum - 10;
  ELSIF sum = 10 THEN
    sum := 0;
  ELSE
    sum := sum + 1;
  END_IF;

  (* A standard function block: toggle the lamp every 500 ms. *)
  blink(IN := NOT lamp, PT := T#500ms);
  IF blink.Q THEN
    lamp := NOT lamp;
  END_IF;
END_PROGRAM

CONFIGURATION Hello
RESOURCE Cpu ON Cpu
  TASK MainTask(INTERVAL := T#10ms, PRIORITY := 1);
  PROGRAM hello WITH MainTask : Main;
END_RESOURCE
END_CONFIGURATION
