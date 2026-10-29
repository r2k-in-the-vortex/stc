(* motor: the root package in the multi-package example.

   It depends on `../motorlib`, selects the configuration, binds two direct
   variables to a demo device, declares retained state, and drives the
   library class/FB through a scan. Build it with `stc build`; the dependency
   is compiled first from its local path and pinned in `stc.lock`.

   `stc` is not a safety PLC and is not responsible for safety. It can control
   real machinery, but safety must be provided by a separate safety system and
   machine design as required by law and regulation. This example drives a
   simulated demo device. *)

PROGRAM Motor
USING motorlib;
VAR
  Ready AT %IX0.0 : BOOL {stc:io {"device":"drive1","channel":0,"direction":"input"}};
  Enable AT %QX0.0 : BOOL {stc:io {"device":"drive1","channel":1,"direction":"output"}};
END_VAR
VAR
  drive : Drive;
  target : Percent;
  state : DriveState;
  display : USTRING[32];
  cycle_count : DINT;
END_VAR
VAR RETAIN
  total_cycles : DINT;
END_VAR
  (* The input comes from the demo device; default it to off in a null run. *)
  IF Ready THEN
    target := 80;
  ELSE
    target := 30;
  END_IF;

  Enable := drive.Start(target);
  state := drive.state;
  display := Describe(state, 'motor');
  cycle_count := cycle_count + 1;
  total_cycles := total_cycles + 1;

  (* Keep the held output bounded for the demo. *)
  IF LEN(display) = 0 THEN
    Enable := FALSE;
  END_IF;
END_PROGRAM

CONFIGURATION MotorDemo
RESOURCE Cpu ON Cpu
  TASK MainTask(INTERVAL := T#10ms, PRIORITY := 1);
  PROGRAM motor WITH MainTask : Motor;
END_RESOURCE
END_CONFIGURATION
