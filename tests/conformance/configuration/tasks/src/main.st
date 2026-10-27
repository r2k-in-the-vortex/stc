VAR_GLOBAL
  fast_count : DINT;
  slow_count : DINT;
END_VAR
PROGRAM Fast
  fast_count := fast_count + 1;
END_PROGRAM
PROGRAM Slow
  slow_count := slow_count + 1;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON CPU
  TASK t_fast (INTERVAL := T#1ms, PRIORITY := 1);
  TASK t_slow (INTERVAL := T#2ms, PRIORITY := 2);
  PROGRAM p_fast WITH t_fast : Fast;
  PROGRAM p_slow WITH t_slow : Slow;
END_RESOURCE
END_CONFIGURATION
