(* M4 smoke example: scalar program with a function call. *)
FUNCTION Add : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  Add := a + b;
END_FUNCTION

PROGRAM Main
VAR
  counter : DINT;
  flag : BOOL;
END_VAR
  counter := Add(counter, 1);
  flag := counter > 3;
END_PROGRAM

CONFIGURATION Default
RESOURCE Cpu ON Cpu
  PROGRAM main_instance : Main;
END_RESOURCE
END_CONFIGURATION
