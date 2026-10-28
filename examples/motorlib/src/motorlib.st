(* motorlib: the reusable package in the motor example.

   It shows the 2013 OOP surface (an interface, an implementing class, and a
   method), a function block with persistent state, an enumeration, a
   subrange, a UTF-8 string function result, and CASE dispatch. The root
   `motor` package depends on this package and supplies the configuration,
   I/O bindings, test suite, and debug/online-change walkthrough. *)

TYPE
  DriveState : (Idle, Running, Faulted);
  Percent : INT(0..100);
END_TYPE

INTERFACE IDrive
METHOD Start : BOOL
VAR_INPUT
  target : Percent;
END_VAR
END_METHOD

METHOD Stop : BOOL
END_METHOD
END_INTERFACE

FUNCTION_BLOCK Ramp
VAR_INPUT
  target : Percent;
END_VAR
VAR_OUTPUT PUBLIC
  value : Percent;
END_VAR
  (* Move toward the target by ten percent per call and clamp. *)
  IF value < target THEN
    value := MIN(value + 10, target);
  ELSIF value > target THEN
    value := MAX(value - 10, target);
  END_IF;
END_FUNCTION_BLOCK

CLASS Drive IMPLEMENTS IDrive
VAR PUBLIC
  state : DriveState;
  speed : Percent;
  label : USTRING[16];
END_VAR
VAR
  ramp : Ramp;
END_VAR

METHOD PUBLIC Start : BOOL
VAR_INPUT
  target : Percent;
END_VAR
  ramp(target := target);
  speed := ramp.value;
  IF speed > 0 THEN
    state := Running;
  ELSE
    state := Idle;
  END_IF;
  Start := state = Running;
END_METHOD

METHOD PUBLIC Stop : BOOL
  state := Idle;
  speed := 0;
  Stop := TRUE;
END_METHOD

END_CLASS

FUNCTION Describe : USTRING[16]
VAR_INPUT
  state : DriveState;
  label : USTRING[16];
END_VAR
  CASE state OF
    Idle:
      Describe := label;
    Running:
      Describe := CONCAT(label, ': running');
    Faulted:
      Describe := CONCAT(label, ': fault');
  ELSE
    Describe := '?';
  END_CASE;
END_FUNCTION
