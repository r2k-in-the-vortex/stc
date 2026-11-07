# Remote debug HIL runbook — WL-1.2

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-11-06  
Status: Pending live HIL on `r2k@ecat`. This runbook is filled during WL-1.2 phase 6; it is not a
safety claim.  
Parent: [Remote-Debug-Threat-Model.md](Remote-Debug-Threat-Model.md)  
Inputs: [Debug and Monitoring](../Architecture/Debug-and-Monitoring.md),
[IDE Integration](../Architecture/IDE-Integration.md),
[CLI and Manifest](../Architecture/CLI-and-Manifest.md)

## 1. Scope

The procedure verifies remote launch and attach against the `ecat-host` reference host described
in [EtherCAT-Runbook.md](EtherCAT-Runbook.md): source sync, remote build, source-fingerprint
identity, first-statement breakpoints on cycle 0, stepping, stack/locals/globals, source mapping,
tunnel drop and reattach, online change, cold download, cancel, and mismatch rejection.

The host receives a packaged Linux `stc`; `stc` and the runtime are never built on the host. The
login user is `r2k`; the procedure uses no `sudo` and does not change IgH, the boot profile, or
EtherCAT state. The remote work directory is `~/stc-wl12`, separate from the `~/stc-wl52`
EtherCAT fixture.

## 2. Host and prerequisites

| Item | Reference value |
| --- | --- |
| Host | `ecat-host`, reachable as `r2k@ecat` |
| OS | Ubuntu Server 24.04.5 LTS, x86-64 |
| Kernel | `6.12.115-ecat-rt` |
| Compiler | `clang` 18.1.3 |
| Remote `stc` | Packaged from a verified CI artifact under `~/stc-wl12` |
| Local tools | OpenSSH client, SSH agent/key, `ecat` in `known_hosts` |
| Work directory | `~/stc-wl12` |

Record `uname -a`, `cat /etc/os-release`, `clang --version`, `stc --version`, and the SHA-256 of
the package before starting.

## 3. Procedure

1. Copy the packaged `stc` to `~/stc-wl12` and verify its SHA-256 without building it.
2. Run a remote launch from a sample project with a breakpoint on the first statement of the
   first program. Confirm the host reports cycle 0 and the frame/locals values.
3. Step, inspect stack, locals, and globals, and confirm source mapping uses the local files.
4. Drop the tunnel and confirm the session ends without automatic reconnect; reattach with a new
   session and re-apply breakpoints.
5. Change a source file and attach with `remoteDeploy=online`; confirm the control deploy and the
   refreshed descriptor fingerprint.
6. Change a source layout incompatibly and confirm the host reports cold-required; retry with
   `remoteDeploy=cold` and confirm the restart, retained-state policy, and reattach.
7. Cancel the QuickPick and confirm the remote host is unchanged and no session starts.
8. Attach with `remoteDeploy=never` after drift and confirm `STC-E-REMOTE-003`.

## 4. Evidence

Record the host/OS/build IDs, commands, logs, produced descriptor fields, and SHA-256 hashes in
this section after the run. The threat model's verification table points here for the live
negative and reattachment evidence.
