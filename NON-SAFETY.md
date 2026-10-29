# Non-safety statement

Profile: `stc-2013-oop-utf8-v1`  
Applies to: `stc` 1.0.0 and its runtime

**`stc` is not a safety PLC and is not responsible for safety in any way.**

`stc` and its runtime **can be used to control real machinery**. They are
general-purpose automation software. What they do not do is provide safety:
safety must be provided by a separate safety system and by machine design
considerations as dictated by applicable laws and regulations. A machine that
uses `stc` for control still needs an independent, certified safety function
(emergency stop, guard monitoring, safe torque off, light curtains, interlocks,
or whatever the risk assessment requires) that does not depend on `stc`,
its runtime, its debugger, or its online-change implementation.

The project makes no SIL, PL, Category, or other safety-integrity claim, and it
has not been designed, verified, or certified for use as a safety function.

The following properties are deliberate and are not defects:

- **Debug can hold outputs indefinitely.** Starting a debug session does not
  stop the cycle, but a stop, breakpoint, step, or force can hold the last
  published outputs for as long as the session remains stopped. The stop/idle
  budget (`[debug].session-timeout`, five minutes by default) bounds a stop and
  resumes the cycle when it expires, but it is not a fail-safe output timeout.
- **Forcing overrides program logic.** A force is applied at a cycle boundary
  and held until released, the session ends, or a generation change occurs.
  Forced inputs and outputs are not a safety interlock.
- **There is no automatic fail-safe state, watchdog output, redundancy, or
  safe-state transition.** The runtime does not force outputs to a de-energized
  state when a client disconnects, a task overruns, or the host is stopped.
  Those functions belong to the separate safety system.
- **Timing is not guaranteed.** The runtime is not a hard real-time system.
  Maintenance work, debug sessions, operating-system scheduling, and I/O
  backends can delay scans. The recorded performance figures are
  informational.
- **Online change is a deployment convenience.** A successful swap preserves
  compatible state and keeps the host running; it is not a safety mechanism and
  does not validate the safety of the changed logic.
- **The compiler is not a safety-qualified toolchain.** Diagnostics,
  reproducibility, and tests reduce risk; they do not certify it.

Anyone using `stc` where a fault could cause injury, environmental damage, or
significant property loss must provide a separate, certified safety layer and
must not rely on `stc`, its debugger, or its online-change implementation for
safety. The control logic may still run on `stc`; the safety function must not.

This statement is repeated in `README.md`, `RELEASE-NOTES.md`,
`KNOWN-LIMITATIONS.md`, `USER-GUIDE.md`, the `stc compliance` output, and every
example README. It may not be softened in later documentation.

See [Conformance Profile](../Specification/01-Conformance-Profile.md) §7,
[Debug and Monitoring](../Specification/Architecture/Debug-and-Monitoring.md)
§17, and decisions D-194 and D-198.
