(* Modbus TCP bench fixture for WL-5.1 phase 4.

   The program copies every Modbus input channel to its output counterpart so
   a network client can write the input tables and read the output tables at
   known zero-based addresses. The copies exercise BYTE, WORD, DWORD, and
   LWORD values without changing them; byte and word order are the backend's
   wire-format concern, not the program's.

   Process-image map (see io/bench.toml for the Modbus address map):
     %IX0.0..%IX0.2   CoilIn0..CoilIn2    coil inputs 0..2
     %IB1..%IB2       ByteInLo/ByteInHi   holding register 0 low/high
     %IW2             WordIn              holding register 2
     %ID2             DwordIn             holding registers 4..5 (low-first)
     %ID3             DwordSwapIn         holding registers 6..7 ("dcba")
     %IL2             LwordIn             holding registers 12..15 (list)
     %QX0.0           CoilOut             coil output 1024
     %QX0.1           DiOut               discrete input 0
     %QB1..%QB2       ByteOutLo/ByteOutHi holding register 8 low/high
     %QW2             WordOut             holding register 10
     %QD2             DwordOut            input registers 0..1 (low-first)
     %QD3             DwordSwapOut        input registers 2..3 ("dcba")
     %QL2             LwordOut            input registers 4..7 (list)

   The modbus-tcp-status device fills its bound %I bytes at the input
   boundary; those variables are read-only mirrors and are not touched here.
   Direct-variable byte offsets: %IBn = byte n, %IWn = bytes 2n..2n+1,
   %IDn = bytes 4n..4n+3, %ILn = bytes 8n..8n+7. *)

VAR_GLOBAL
  (* Coil inputs: the client writes these with FC5/FC15. *)
  CoilIn0 AT %IX0.0 : BOOL;
  CoilIn1 AT %IX0.1 : BOOL;
  CoilIn2 AT %IX0.2 : BOOL;

  (* 8-bit inputs sharing holding register 0 (low and high bytes). *)
  ByteInLo AT %IB1 : BYTE;
  ByteInHi AT %IB2 : BYTE;

  (* 16-, 32-, and 64-bit inputs on the holding-register table. *)
  WordIn AT %IW2 : WORD;
  DwordIn AT %ID2 : DWORD;
  DwordSwapIn AT %ID3 : DWORD;
  LwordIn AT %IL2 : LWORD;

  (* Coil and discrete-input outputs. *)
  CoilOut AT %QX0.0 : BOOL;
  DiOut AT %QX0.1 : BOOL;

  (* 8-bit outputs sharing holding register 8. *)
  ByteOutLo AT %QB1 : BYTE;
  ByteOutHi AT %QB2 : BYTE;

  (* 16-, 32-, and 64-bit outputs on the holding- and input-register tables. *)
  WordOut AT %QW2 : WORD;
  DwordOut AT %QD2 : DWORD;
  DwordSwapOut AT %QD3 : DWORD;
  LwordOut AT %QL2 : LWORD;

  (* modbus-tcp-status layout 1 mirrors. Byte offsets: %IWn = 2n,
     %IDn = 4n; the listening bit is byte 24 bit 0. *)
  MbListening AT %IX24.0 : BOOL;        (* server0.listening       bytes 24 *)
  MbListeners AT %IW13 : UINT;          (* server0.listeners       bytes 26..27 *)
  MbBoundListeners AT %IW14 : UINT;     (* server0.bound_listeners bytes 28..29 *)
  MbConnections AT %IW15 : UINT;        (* server0.connections     bytes 30..31 *)
  MbRequests AT %ID8 : UDINT;           (* server0.requests        bytes 32..35 *)
  MbWrites AT %ID9 : UDINT;             (* server0.writes          bytes 36..39 *)
  MbExceptions AT %ID10 : UDINT;        (* server0.exceptions      bytes 40..43 *)
  MbMalformed AT %ID11 : UDINT;         (* server0.malformed       bytes 44..47 *)
  MbLastError AT %ID12 : UDINT;         (* server0.last_error      bytes 48..51 *)
END_VAR

PROGRAM Main
VAR
END_VAR
  (* Mirror every input channel to its output channel. The values are not
     modified, so the client's written numbers round-trip exactly. *)
  CoilOut := CoilIn0;
  DiOut := CoilIn1;

  ByteOutLo := ByteInLo;
  ByteOutHi := ByteInHi;

  WordOut := WordIn;
  DwordOut := DwordIn;
  DwordSwapOut := DwordSwapIn;
  LwordOut := LwordIn;
END_PROGRAM

CONFIGURATION ModbusBench
RESOURCE Cpu ON Cpu
  TASK MainTask(INTERVAL := T#1ms, PRIORITY := 1);
  PROGRAM main WITH MainTask : Main;
END_RESOURCE
END_CONFIGURATION
