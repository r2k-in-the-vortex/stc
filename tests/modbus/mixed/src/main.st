(* Mixed DIEWU EtherCAT + Modbus TCP bench program (WL-5.1 phase 4).

   Two hardware backends are active in one coordinator: the DIEWU
   EtherCAT-IO slave and the Modbus TCP server. The program keeps both data
   paths moving without wiring them together:

     * every DIEWU input is mirrored to its output (open-ended, unwired);
     * the Modbus client's command word 0 is incremented to holding register 1;
     * the Modbus client's coil 0 is copied to coil 1.

   The cross-backend signal path is deliberately absent: no external
   Modbus-to-EtherCAT connection device exists on the bench. The coordinator's
   mixed mock test proves cross-backend routing over one image; this fixture
   proves concurrent real backends and healthy status.

   Process-image map (see io/bench.toml):
     %IX0.0..%IX1.7   In0..In15       DIEWU inputs 0x3001:01/02
     %QX0.0..%QX1.7   Out0..Out15     DIEWU outputs 0x3101:01/02
     %IW20            BenchCmd        Modbus holding register 0
     %IX42.0          BenchBit        Modbus coil 0
     %QW20            BenchSeen       Modbus holding register 1
     %QX42.0          BenchOut        Modbus coil 1

   EtherCAT status occupies %I bytes 4..38 (same layout as tests/ethercat/
   diewu). Modbus status occupies %I bytes 48..75. Data channels sit outside
   both status blocks. %IWn = bytes 2n..2n+1, %IDn = bytes 4n..4n+3. *)

VAR_GLOBAL
  (* DIEWU inputs, byte 0 and byte 1 of the input area. *)
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

  (* DIEWU outputs, byte 0 and byte 1 of the output area. *)
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

  (* EtherCAT status mirror. Byte offsets: %IWn = 2n, %IDn = 4n. *)
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

  (* Modbus data channels. *)
  BenchCmd AT %IW20 : UINT;    (* holding register 0, bytes 40..41 *)
  BenchBit AT %IX42.0 : BOOL;  (* coil 0, byte 42, bit 0 *)
  BenchSeen AT %QW20 : UINT;   (* holding register 1, bytes 40..41 *)
  BenchOut AT %QX42.0 : BOOL;  (* coil 1, byte 42, bit 0 *)

  (* Modbus status mirror (server0). Byte offsets: %IWn = 2n, %IDn = 4n. *)
  MbListening AT %IX48.0 : BOOL;        (* server0.listening       byte 48 *)
  MbListeners AT %IW25 : UINT;          (* server0.listeners       bytes 50..51 *)
  MbBoundListeners AT %IW26 : UINT;     (* server0.bound_listeners bytes 52..53 *)
  MbConnections AT %IW27 : UINT;        (* server0.connections     bytes 54..55 *)
  MbRequests AT %ID14 : UDINT;          (* server0.requests        bytes 56..59 *)
  MbWrites AT %ID15 : UDINT;            (* server0.writes          bytes 60..63 *)
  MbExceptions AT %ID16 : UDINT;        (* server0.exceptions      bytes 64..67 *)
  MbMalformed AT %ID17 : UDINT;         (* server0.malformed       bytes 68..71 *)
  MbLastError AT %ID18 : UDINT;         (* server0.last_error      bytes 72..75 *)
END_VAR

VAR_GLOBAL
  Counter : UDINT;
END_VAR

PROGRAM Main
VAR
END_VAR
  (* Open-ended EtherCAT path: inputs to outputs. *)
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
  Out14 := In14;
  Out15 := In15;

  (* Modbus path: prove the PLC scan keeps serving the network client while
     the EtherCAT frames run. *)
  BenchSeen := BenchCmd + 1;
  BenchOut := BenchBit;

  Counter := Counter + 1;
END_PROGRAM

CONFIGURATION MixedBench
RESOURCE Cpu ON Cpu
  TASK MainTask(INTERVAL := T#1ms, PRIORITY := 1);
  PROGRAM main WITH MainTask : Main;
END_RESOURCE
END_CONFIGURATION
