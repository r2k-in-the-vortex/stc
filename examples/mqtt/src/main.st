(* mqtt: process-image I/O over an external MQTT 5.0 broker.

   `Start` and `Setpoint` are inputs decoded from broker messages. `Running`
   and `Speed` are outputs encoded into broker messages. The `BrokerStatus`
   device exposes the optional layout-1 connection and diagnostic channels.

   See README.md for the Mosquitto and MQTTX Web walkthrough.

   `stc` is not a safety PLC and is not responsible for safety. It can control
   real machinery, but safety must be provided by a separate safety system and
   machine design as required by law and regulation. MQTT is asynchronous,
   non-real-time, and carries untrusted network input. *)

(* Some addresses are offset (`Setpoint` at `%ID4` rather than `%ID0`):
   the current image/runtime rules treat overlapping direct ranges as aliases
   and reject overlapping bound ranges. Channels bind by variable name, so
   the numbers are labels. The follow-up is WL-23 in FutureWishlist.md. *)
VAR_GLOBAL
  Start AT %IX0.0 : BOOL;
  Setpoint AT %ID4 : REAL;

  Running AT %QX0.0 : BOOL;
  Speed AT %QD4 : REAL;

  Connected AT %IX8.0 : BOOL;
  Reconnects AT %ID12 : UDINT;
  MessagesIn AT %ID16 : UDINT;
  Stale AT %IX8.1 : BOOL;

  StatusConnected AT %QX8.0 : BOOL;
  StatusReconnects AT %QD12 : UDINT;
  StatusMessagesIn AT %QD16 : UDINT;
  StatusStale AT %QX8.1 : BOOL;
END_VAR

(* Keep a requested speed inside the example's safe demonstration range.
   Change this function to see how a source edit is handled by `stc run --watch`. *)
FUNCTION ClampSpeed : REAL
VAR_INPUT
  requested : REAL;
END_VAR
  IF requested < 0.0 THEN
    ClampSpeed := 0.0;
  ELSIF requested > 100.0 THEN
    ClampSpeed := 100.0;
  ELSE
    ClampSpeed := requested;
  END_IF;
END_FUNCTION

PROGRAM Main
VAR
  cycles : UDINT;
END_VAR
  Running := Start;
  IF Start THEN
    Speed := ClampSpeed(Setpoint);
  ELSE
    Speed := 0.0;
  END_IF;

  (* Publish the status device values as ordinary retained outputs so a broker
     client such as MQTTX Web can observe them. They are diagnostics, not
     safety signals. *)
  StatusConnected := Connected;
  StatusReconnects := Reconnects;
  StatusMessagesIn := MessagesIn;
  StatusStale := Stale;
  cycles := cycles + 1;
END_PROGRAM

CONFIGURATION MqttExample
RESOURCE Cpu ON Cpu
  TASK MainTask(INTERVAL := T#20ms, PRIORITY := 1);
  PROGRAM main WITH MainTask : Main;
END_RESOURCE
END_CONFIGURATION
