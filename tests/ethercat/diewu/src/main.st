(* DIEWU 16I/16O open-ended HIL fixture for WL-5.2 phase 3.

   The program is deliberately trivial: it copies the 14 directly bound
   inputs to their outputs, toggles a state bit when In14 is set, and counts
   scans. Nothing here is compute-bound; the point of the run is that IgH
   accepts and replies to every process-data frame while the slave is in OP.

   Process-image map (see io/bench.toml for the PDO channel map):
     %IX0.0..%IX0.7   In0..In7     DIEWU inputs 0x3001:01 bits 0..7
     %IX1.0..%IX1.7   In8..In15    DIEWU inputs 0x3001:02 bits 0..7
     %IX2.0           Shadow1      sink of the buffered PLC -> PLC route
     %QX0.0..%QX0.7   Out0..Out7   DIEWU outputs 0x3101:01 bits 0..7
     %QX1.0..%QX1.5   Out8..Out13  DIEWU outputs 0x3101:02 bits 0..5
     OUT14 (0x3101:02 bit 6) is driven by the route Out0 -> OUT14.
     OUT15 (0x3101:02 bit 7) is driven by the route IN15 -> OUT15.

   Status variables are plain %I image words filled by the ethercat-status
   device at the input boundary. %IWn addresses word n (byte 2n) and %IDn
   addresses dword n (byte 4n); see the comments next to each declaration. *)

VAR_GLOBAL
  (* DIEWU inputs: byte 0 and byte 1 of the input area. *)
  In0 AT %IX0.0 : BOOL;
  In1 AT %IX0.1 : BOOL;
  In2 AT %IX0.2 : BOOL;
  In3 AT %IX0.3 : BOOL;
  In4 AT %IX0.4 : BOOL;
  In5 AT %IX0.5 : BOOL;
  In6 AT %IX0.6 : BOOL;
  In7 AT %IX0.7 : BOOL;
  In8 AT %IX1.0 : BOOL;
  In9 AT %IX1.1 : BOOL;
  In10 AT %IX1.2 : BOOL;
  In11 AT %IX1.3 : BOOL;
  In12 AT %IX1.4 : BOOL;
  In13 AT %IX1.5 : BOOL;
  In14 AT %IX1.6 : BOOL;
  In15 AT %IX1.7 : BOOL;

  (* Buffered PLC -> PLC route sink. *)
  Shadow1 AT %IX2.0 : BOOL;

  (* DIEWU outputs. *)
  Out0 AT %QX0.0 : BOOL;
  Out1 AT %QX0.1 : BOOL;
  Out2 AT %QX0.2 : BOOL;
  Out3 AT %QX0.3 : BOOL;
  Out4 AT %QX0.4 : BOOL;
  Out5 AT %QX0.5 : BOOL;
  Out6 AT %QX0.6 : BOOL;
  Out7 AT %QX0.7 : BOOL;
  Out8 AT %QX1.0 : BOOL;
  Out9 AT %QX1.1 : BOOL;
  Out10 AT %QX1.2 : BOOL;
  Out11 AT %QX1.3 : BOOL;
  Out12 AT %QX1.4 : BOOL;
  Out13 AT %QX1.5 : BOOL;

  (* Status device mirror. Byte offsets: %IWn = 2n, %IDn = 4n. *)
  StatusPhase AT %IW2 : UINT;               (* bytes 4..5 *)
  StatusMasterActive AT %IX6.0 : BOOL;      (* byte 6, bit 0 *)
  StatusSlavesResponding AT %IW4 : UINT;    (* bytes 8..9 *)
  StatusLinkUp AT %IX10.0 : BOOL;           (* byte 10, bit 0 *)
  StatusLastError AT %ID3 : UDINT;          (* bytes 12..15 *)
  StatusRecoveries AT %ID4 : UDINT;         (* bytes 16..19 *)
  StatusWcState AT %IW10 : UINT;            (* bytes 20..21 *)
  StatusWorkingCounter AT %IW11 : UINT;     (* bytes 22..23 *)
  StatusInputStale AT %IX24.0 : BOOL;       (* byte 24, bit 0 *)
  StatusExchanges AT %ID7 : UDINT;          (* bytes 28..31 *)
  StatusFaults AT %ID8 : UDINT;             (* bytes 32..35 *)
  StatusSlaveAlState AT %IW18 : UINT;       (* bytes 36..37 *)
  StatusSlaveOnline AT %IX38.0 : BOOL;      (* byte 38, bit 0 *)
  StatusSlaveOperational AT %IX38.1 : BOOL; (* byte 38, bit 1 *)
END_VAR

VAR_GLOBAL
  Counter : UDINT;
  Toggle : BOOL;
END_VAR

PROGRAM Main
VAR
END_VAR
  (* Open-ended I/O: mirror the bound inputs to the bound outputs. *)
  Out0 := In0;
  Out1 := In1;
  Out2 := In2;
  Out3 := In3;
  Out4 := In4;
  Out5 := In5;
  Out6 := In6;
  Out7 := In7;
  Out8 := In8;
  Out9 := In9;
  Out10 := In10;
  Out11 := In11;
  Out12 := In12;
  Out13 := In13;

  (* State that survives across scans; In14 toggles it. *)
  IF In14 THEN
    Toggle := NOT Toggle;
  END_IF;

  Counter := Counter + 1;
END_PROGRAM

CONFIGURATION DiewuHil
RESOURCE Cpu ON Cpu
  TASK MainTask(INTERVAL := T#1ms, PRIORITY := 1);
  PROGRAM main WITH MainTask : Main;
END_RESOURCE
END_CONFIGURATION
