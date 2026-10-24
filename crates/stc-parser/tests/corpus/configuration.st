CONFIGURATION Plant
VAR_GLOBAL
  running : BOOL;
END_VAR
RESOURCE Line ON PLC
VAR_GLOBAL
  local : INT;
END_VAR
  TASK Fast(INTERVAL := T#10ms, PRIORITY := 1);
  TASK Slow(SINGLE := T#100ms);
  PROGRAM Main WITH Fast : MainProgram (cycle := 1, out => %QW0);
  PROGRAM Backup WITH Slow : BackupProgram;
END_RESOURCE
VAR_ACCESS
  running_path : Plant.running : BOOL READ_WRITE;
END_VAR
VAR_CONFIG
  Main.Timer.Preset AT %MW10 : INT := 5;
END_VAR
END_CONFIGURATION
