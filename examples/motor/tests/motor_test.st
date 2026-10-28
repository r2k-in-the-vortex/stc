(* Tests for the motor example. They live in the root package and use the
   `motorlib` dependency through its package namespace. `stc test` builds each
   {stc:test} POU into its own process; production artifacts never include
   this file. *)

{stc:test}
FUNCTION RampReachesTarget : BOOL
VAR
  ramp : motorlib.Ramp;
  i : DINT;
END_VAR
  FOR i := 1 TO 10 DO
    ramp(target := 50);
  END_FOR;
  STC_ASSERT(ramp.value = 50, "the ramp should reach its target");
  RampReachesTarget := TRUE;
END_FUNCTION

{stc:test}
FUNCTION DriveStarts : BOOL
VAR
  drive : motorlib.Drive;
  started : BOOL;
END_VAR
  started := drive.Start(40);
  STC_ASSERT(started, "Drive.Start should report running after one ramp step");
  STC_ASSERT(drive.state = motorlib.Running, "the drive state should be Running");
  STC_ASSERT(drive.speed > 0, "the drive speed should advance");
  DriveStarts := TRUE;
END_FUNCTION

{stc:test}
FUNCTION DescribeUsesUtf8Text : BOOL
VAR
  text : USTRING[32];
END_VAR
  text := motorlib.Describe(motorlib.Running, 'motor');
  STC_ASSERT(text = 'motor: running', "Describe should append the state suffix");
  STC_ASSERT(LEN_CODE_UNIT(text) = 14, "the UTF-8 length should count the ASCII interchange bytes");
  DescribeUsesUtf8Text := TRUE;
END_FUNCTION

{stc:test}
FUNCTION DriveStops : BOOL
VAR
  drive : motorlib.Drive;
END_VAR
  drive.Start(50);
  drive.Stop();
  STC_ASSERT(drive.state = motorlib.Idle, "Stop should return the drive to Idle");
  STC_ASSERT(drive.speed = 0, "Stop should clear the speed");
  DriveStops := TRUE;
END_FUNCTION
