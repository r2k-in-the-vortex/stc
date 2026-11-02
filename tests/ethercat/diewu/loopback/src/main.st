(* DIEWU 16I/16O loopback all-points test for WL-5.2 phase 6.

   The bench wires OUT1..OUT16 to IN1..IN16 (fixture channels OUT0..OUT15 to
   IN0..IN15) and ties the input commons S1..S4 to the positive output supply.
   An NPN sinking output is on when its IN terminal is pulled low, and the
   input opto is the load between S and IN, so output channel N on reads back
   as input channel N on.

   The program walks a one-hot pattern across OUT0..OUT15. For each channel it
   drives that output for 20 scans (20 ms at the 1 ms task), then samples all
   16 inputs: channel N passes when the input mask equals exactly bit N, that
   is, IN(N+1) reads 1 and every other input reads 0. A mismatch sets bit N of
   FailMask. After channel 15 the outputs are driven off, Pass is latched from
   FailMask = 0, and Done is set.

   Hold is a manual fault-observation mode used by the phase-6 fault
   procedure. When it is TRUE the all-points state machine stops advancing and
   all 16 outputs are driven on, so the runtime's output image (OutputMask)
   and the last received input image (InputMask) can be read while a physical
   fault is injected. Set Hold only after Done; the procedure in PHASE6.md
   does that through var.write.

   This is a functional loopback test, not a safety function. It verifies the
   module output drivers against the module input optos only.

   Process-image map (see io/bench.toml for the PDO channel map):
     %IX0.0..%IX1.7   In0..In15    DIEWU inputs 0x3001:01/0x3001:02
     %QX0.0..%QX1.7   Out0..Out15  DIEWU outputs 0x3101:01/0x3101:02

   Status variables are plain %I image words filled by the ethercat-status
   device at the input boundary. %IWn addresses word n (byte 2n) and %IDn
   addresses dword n (byte 4n); bytes 2..3 are unused in this project because
   it has no PLC-in shadow variable. *)

VAR_GLOBAL
  (* DIEWU inputs, terminals IN1..IN16. *)
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

  (* DIEWU outputs, terminals OUT1..OUT16. *)
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
  Out14 AT %QX1.6 : BOOL;
  Out15 AT %QX1.7 : BOOL;

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
  (* All-points test state and results. The loopback harness reads these
     through var.read on DIEWU-LOOPBACK::DONE, ::PASS, and ::FAILMASK. *)
  Phase : UINT;          (* 0 warmup, 1 drive/settle, 2 sample, 3 done *)
  Channel : UINT;        (* channel being driven, 16 after the last sample *)
  Settle : UINT;         (* scans spent in the current drive or warmup *)
  InputMask : WORD;      (* all 16 inputs this scan, bit N = InN *)
  OutputMask : WORD;     (* all 16 outputs this scan, bit N = OutN *)
  ObservedMask : WORD;   (* bit N set when InN read 1 at channel N sample *)
  FailMask : WORD;       (* bit N set when channel N failed *)
  Pass : BOOL;           (* FailMask = 0 after the walk *)
  Done : BOOL;           (* all 16 channels sampled and Pass latched *)
  Hold : BOOL;           (* manual fault-observation hold, set via var.write *)
  ScanCount : UDINT;     (* scans since start; advances during a fault *)
END_VAR

PROGRAM Main
VAR
  ChannelBit : WORD;
END_VAR
  (* Default output state: every output off. *)
  Out0 := FALSE;
  Out1 := FALSE;
  Out2 := FALSE;
  Out3 := FALSE;
  Out4 := FALSE;
  Out5 := FALSE;
  Out6 := FALSE;
  Out7 := FALSE;
  Out8 := FALSE;
  Out9 := FALSE;
  Out10 := FALSE;
  Out11 := FALSE;
  Out12 := FALSE;
  Out13 := FALSE;
  Out14 := FALSE;
  Out15 := FALSE;

  (* Read all 16 inputs into a mask before the outputs change. *)
  InputMask := WORD#0;
  IF In0 THEN
    InputMask := InputMask OR WORD#1;
  END_IF;
  IF In1 THEN
    InputMask := InputMask OR WORD#2;
  END_IF;
  IF In2 THEN
    InputMask := InputMask OR WORD#4;
  END_IF;
  IF In3 THEN
    InputMask := InputMask OR WORD#8;
  END_IF;
  IF In4 THEN
    InputMask := InputMask OR WORD#16;
  END_IF;
  IF In5 THEN
    InputMask := InputMask OR WORD#32;
  END_IF;
  IF In6 THEN
    InputMask := InputMask OR WORD#64;
  END_IF;
  IF In7 THEN
    InputMask := InputMask OR WORD#128;
  END_IF;
  IF In8 THEN
    InputMask := InputMask OR WORD#256;
  END_IF;
  IF In9 THEN
    InputMask := InputMask OR WORD#512;
  END_IF;
  IF In10 THEN
    InputMask := InputMask OR WORD#1024;
  END_IF;
  IF In11 THEN
    InputMask := InputMask OR WORD#2048;
  END_IF;
  IF In12 THEN
    InputMask := InputMask OR WORD#4096;
  END_IF;
  IF In13 THEN
    InputMask := InputMask OR WORD#8192;
  END_IF;
  IF In14 THEN
    InputMask := InputMask OR WORD#16384;
  END_IF;
  IF In15 THEN
    InputMask := InputMask OR WORD#32768;
  END_IF;

  IF Hold THEN
    (* Fault observation: hold all outputs on and freeze the walk. *)
    Out0 := TRUE;
    Out1 := TRUE;
    Out2 := TRUE;
    Out3 := TRUE;
    Out4 := TRUE;
    Out5 := TRUE;
    Out6 := TRUE;
    Out7 := TRUE;
    Out8 := TRUE;
    Out9 := TRUE;
    Out10 := TRUE;
    Out11 := TRUE;
    Out12 := TRUE;
    Out13 := TRUE;
    Out14 := TRUE;
    Out15 := TRUE;
  ELSE
    CASE Phase OF
      0:
        (* Warmup: outputs off for 20 scans after the slave reaches OP. *)
        IF Settle < 20 THEN
          Settle := Settle + 1;
        ELSE
          Phase := 1;
          Settle := 0;
          Channel := 0;
        END_IF;
      1:
        (* Drive the current channel one-hot and wait for settling. *)
        CASE Channel OF
          0: Out0 := TRUE;
          1: Out1 := TRUE;
          2: Out2 := TRUE;
          3: Out3 := TRUE;
          4: Out4 := TRUE;
          5: Out5 := TRUE;
          6: Out6 := TRUE;
          7: Out7 := TRUE;
          8: Out8 := TRUE;
          9: Out9 := TRUE;
          10: Out10 := TRUE;
          11: Out11 := TRUE;
          12: Out12 := TRUE;
          13: Out13 := TRUE;
          14: Out14 := TRUE;
          15: Out15 := TRUE;
        END_CASE;
        IF Settle < 20 THEN
          Settle := Settle + 1;
        ELSE
          Phase := 2;
        END_IF;
      2:
        (* Sample: InputMask must equal exactly the driven channel's bit. *)
        ChannelBit := SHL(WORD#1, Channel);
        IF InputMask = ChannelBit THEN
          ObservedMask := ObservedMask OR ChannelBit;
        ELSE
          FailMask := FailMask OR ChannelBit;
          ObservedMask := ObservedMask OR (InputMask AND ChannelBit);
        END_IF;
        Channel := Channel + 1;
        Settle := 0;
        IF Channel >= 16 THEN
          Phase := 3;
        ELSE
          Phase := 1;
        END_IF;
      3:
        (* Finalize once: outputs are off, latch the result. *)
        IF NOT Done THEN
          Pass := FailMask = WORD#0;
          Done := TRUE;
        END_IF;
    END_CASE;
  END_IF;

  (* Mirror the outputs the program is driving. *)
  OutputMask := WORD#0;
  IF Out0 THEN
    OutputMask := OutputMask OR WORD#1;
  END_IF;
  IF Out1 THEN
    OutputMask := OutputMask OR WORD#2;
  END_IF;
  IF Out2 THEN
    OutputMask := OutputMask OR WORD#4;
  END_IF;
  IF Out3 THEN
    OutputMask := OutputMask OR WORD#8;
  END_IF;
  IF Out4 THEN
    OutputMask := OutputMask OR WORD#16;
  END_IF;
  IF Out5 THEN
    OutputMask := OutputMask OR WORD#32;
  END_IF;
  IF Out6 THEN
    OutputMask := OutputMask OR WORD#64;
  END_IF;
  IF Out7 THEN
    OutputMask := OutputMask OR WORD#128;
  END_IF;
  IF Out8 THEN
    OutputMask := OutputMask OR WORD#256;
  END_IF;
  IF Out9 THEN
    OutputMask := OutputMask OR WORD#512;
  END_IF;
  IF Out10 THEN
    OutputMask := OutputMask OR WORD#1024;
  END_IF;
  IF Out11 THEN
    OutputMask := OutputMask OR WORD#2048;
  END_IF;
  IF Out12 THEN
    OutputMask := OutputMask OR WORD#4096;
  END_IF;
  IF Out13 THEN
    OutputMask := OutputMask OR WORD#8192;
  END_IF;
  IF Out14 THEN
    OutputMask := OutputMask OR WORD#16384;
  END_IF;
  IF Out15 THEN
    OutputMask := OutputMask OR WORD#32768;
  END_IF;

  ScanCount := ScanCount + 1;
END_PROGRAM

CONFIGURATION Loopback
RESOURCE Cpu ON Cpu
  TASK MainTask(INTERVAL := T#1ms, PRIORITY := 1);
  PROGRAM main WITH MainTask : Main;
END_RESOURCE
END_CONFIGURATION
