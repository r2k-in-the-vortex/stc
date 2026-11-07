# Remote debug HIL runbook — WL-1.2

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-11-07  
Status: Live HIL complete on `r2k@ecat`. This document records host-specific evidence; it is not a
safety claim.  
Parent: [Remote-Debug-Threat-Model.md](Remote-Debug-Threat-Model.md)  
Inputs: [Debug and Monitoring](../Architecture/Debug-and-Monitoring.md),
[IDE Integration](../Architecture/IDE-Integration.md),
[CLI and Manifest](../Architecture/CLI-and-Manifest.md)

## 1. Scope

The procedure verifies remote launch and attach against the `ecat-host` reference host described
in [EtherCAT-Runbook.md](EtherCAT-Runbook.md): source sync, remote build, source-fingerprint
identity, first-statement breakpoints on cycle 0, stepping, stack/globals/function locals, source
mapping, tunnel drop and reattach, online change, cold download, cancel, and mismatch rejection.

The host receives a packaged Linux `stc`; `stc` and the runtime are never built on the host. The
login user is `r2k`; the procedure uses no `sudo` and does not change IgH, the boot profile, or
EtherCAT state. The remote work directory is `~/stc-wl12`, separate from the `~/stc-wl52`
EtherCAT fixture.

## 2. Host and prerequisites

| Item | Recorded value |
| --- | --- |
| Host | `ecat-host`, reachable as `r2k@ecat` |
| OS | Ubuntu Server 24.04.5 LTS, x86-64 |
| Kernel | `6.12.115-ecat-rt` (`PREEMPT_RT`), x86-64 |
| Compiler on host | `clang` 18.1.3 |
| Remote package | `~/stc-wl12/stc-735f434`, built from commit `735f434` CI artifact |
| Remote `stc` SHA-256 | `f7c0884347c8cb6e5618aa2a5312fa855299d093ac73071424a53580bdbfe608` |
| Local driver | `stc` 2.0.0 debug build from the same branch |
| Work directory | `~/stc-wl12` |

The package was copied with `scp`, verified with `sha256sum -c SHA256SUMS`, and made executable
without building anything on the host.

## 3. Remote launch

Command (development host, project `examples/hello`):

```sh
stc debug --remote r2k@ecat \
  --remote-dir '~/stc-wl12' \
  --remote-stc '/home/r2k/stc-wl12/stc-735f434/bin/stc' \
  --manifest-path examples/hello/stc.toml \
  --break 'src/main.st:35' \
  --batch launch.jsonl --message-format json
```

The launch synced the mirror, built remotely, started `stc serve --start-paused`, connected
through the tunnel, and released the gate after the breakpoint was applied.

Recorded results:

- `session.start` reported `cycle: 0`.
- The breakpoint at `src/main.st:35` reported `bound: true` with checkpoint `3`.
- `MAIN.I` and `MAIN.SUM` read `0` at the stop, before the first scan (the source comment used for
  the launch batch did not shift line 35).
- `stack.list` reported the `MAIN` frame, source `src/main.st`, with the expected optimized-out
  tail warning.
- Step-into moved to the `Counter` body (`src/main.st:24`); `stack.list` then reported the
  `stc_fb_body_HELLO_n_COUNTER` frame and the `MAIN` caller.
- Program/FB body frames have no frame-table locals (KNOWN-LIMITATIONS F-13); breaking inside
  `FUNCTION AddTwo` (`src/main.st:14`) returned function locals `A = 1` and `B = 1` through
  `frame.read`, and `MAIN.SUM` read `1` through `var.read`.
- Exit code was `0`; the launch-owned serve was stopped at teardown.

## 4. Attach-time online change

A persistent `stc serve` was started from the original `hello` project. A local copy with a
source-only comment change was attached with:

```sh
stc debug --attach --remote r2k@ecat \
  --remote-dir '~/stc-wl12' \
  --remote-stc '/home/r2k/stc-wl12/stc-735f434/bin/stc' \
  --remote-deploy online \
  --break 'src/main.st:35' \
  --batch attach.jsonl --message-format json
```

Recorded results:

- `stc remote status` first reported `drift: false`, then `drift: true` with `online: true` after
  the comment change.
- The attach performed the control deploy; `session.start` reported `generation: 2`.
- The breakpoint bound and stopped, and the session ended cleanly with exit code `0`.
- The online-deployed `stc serve` remained running for the next attach.

## 5. Cold download

After an incompatible source change, an attach with `--remote-deploy cold` performed a stop,
candidate build, restart from the candidate directory, tunnel attach, and breakpoint stop.

Recorded results:

- The new build identity differed (`0xeeff6439…` versus the online build `0x7ca3676a…`).
- `session.start` reported `generation: 1` for the cold-restarted serve.
- The breakpoint bound at `src/main.st:35` and the session exited `0`.
- The restarted `stc serve` remained running after the attach (session descriptor still present).

## 6. Cancel and mismatch

- With `remoteDeploy=never` and a changed local source, attach exited `2` with
  `STC-E-REMOTE-003` before opening a tunnel; the running serve was unchanged.
- `remoteDeploy=ask` with drift returns the same error and asks the caller to choose `online` or
  `cold`; the extension intercepts this with the Online/Cold/Cancel QuickPick. Cancel returns
  without starting a debug session and performs no deploy. The QuickPick decision logic is
  covered by `editors/vscode/test/remote-deploy.test.mjs`; the GUI click itself is not automated
  in this runbook.

## 7. Tunnel drop and reattach

A persistent serve was attached in interactive mode. The local OpenSSH tunnel process was killed
(`ssh -N -T -L …`); the serve stayed alive.

Recorded results:

- The debugger reported `STC-E-DEBUG-020` (“the debug connection failed: … forcibly closed …”)
  and exited without automatic reconnect.
- The remote serve and its descriptor remained present.
- A new attach to the same serve bound the breakpoint at `src/main.st:35`, stopped, and exited
  `0`, confirming explicit reattach with breakpoints re-applied.

Stopping the remote serve during an attached session produced a `session-ended` event with
`reason: "shutdown"` and ended the session without reconnection.

## 8. Evidence summary

| Check | Result |
| --- | --- |
| Remote build and source-fingerprint match | Pass |
| First-statement breakpoint before cycle 0 | Pass (`cycle: 0`, `MAIN.I = 0`, `MAIN.SUM = 0`) |
| Stepping and stack | Pass (step into `Counter`; two real frames plus optimized-out tail) |
| Function locals | Pass (`A = 1`, `B = 1` in `AddTwo`) |
| Globals | Pass (`var.read` for `MAIN.I` / `MAIN.SUM`) |
| Source mapping | Pass (`src/main.st` lines 14, 24, 35, 36) |
| Online change | Pass (generation 2, breakpoint, clean exit) |
| Cold download/restart | Pass (new build ID, serve left running, reattach works) |
| Cancel / mismatch | Mismatch pass; cancel decision covered by extension unit tests |
| Tunnel drop and reattach | Pass (`STC-E-DEBUG-020`, no reconnect, explicit reattach) |

The live run used packaged artifacts and a separate work directory. It did not change IgH, the
EtherCAT state, the boot profile, or the host time base, and it makes no hard-real-time or safety
claim.
