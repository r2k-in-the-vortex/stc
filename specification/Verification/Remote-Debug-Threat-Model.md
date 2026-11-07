# Remote debug threat model — WL-1.2

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-11-06  
Status: Design and verification record for the WL-1.2 SSH-tunnel transport. This document is not a
safety claim; debugging remains an availability feature.  
Parent: [Debug and Monitoring](../Architecture/Debug-and-Monitoring.md)  
Related: [IDE Integration](../Architecture/IDE-Integration.md),
[CLI and Manifest](../Architecture/CLI-and-Manifest.md),
[Remote-Debug-Runbook.md](Remote-Debug-Runbook.md)

## 1. Scope and transport

WL-1.2 lets the development host drive a runtime on another host without adding a network-facing
debug listener or a protocol revision. The transport is:

1. `stc serve` starts the ordinary loopback `socket` debug server and a control endpoint on the
   remote host and writes a user-only `stc-remote/1` session descriptor.
2. The local driver opens an OpenSSH local port-forward
   (`ssh -N -T -L 127.0.0.1:<local>:127.0.0.1:<remote>`) to that loopback endpoint.
3. The ordinary `stc-debug/1` client connects to the local end of the tunnel with the token from
   the remote endpoint file.

Encryption, peer authentication, and user authentication are delegated to OpenSSH. This model
covers the additional driver and host surfaces that WL-1.2 adds. It does not re-audit OpenSSH,
the operating system, or the runtime's existing debug protocol.

## 2. Assets

| Asset | Where it lives | Why it matters |
| --- | --- | --- |
| Debug token | Remote endpoint file, read by the local driver | Authorizes `stc-debug/1` requests, including writes and force when granted |
| Control nonce | Remote host-only file | Authorizes runtime control (`stop`, `deploy`, `download`) |
| Project sources and `stc.lock` | Local project; mirrored under the remote work directory | Intellectual property and the basis of the source fingerprint |
| Built module and GOT manifest | Remote work directory | Executable controller logic and its compatibility plan |
| Retained state | Remote work directory | Process values that can affect machinery after a restart |
| SSH private key / agent | Development host | Authenticates every remote operation |
| `known_hosts` | Development host | Pins the remote host identity |

The runtime's debug and control endpoints remain the authority for authorization. The descriptor
contains no token or nonce; the driver never prints either value.

## 3. Trust boundaries

| Boundary | Trusted for | Not trusted for |
| --- | --- | --- |
| Development host user | Choosing the target, project, and deployment action | Resisting a compromised local account |
| OpenSSH client and server | Confidentiality, integrity, server and user authentication | Application-level authorization |
| Remote login account | Reading its own files and running its own processes | Protecting against a compromised account or root |
| Remote work directory | Private files (`0600`) under the login user | Multi-tenant isolation |
| Runtime debug server | Token checks, grants, audit log | Network exposure; it binds loopback only |
| Runtime control server | Nonce checks and operation allowlist | Exposure outside the user-only socket |

The security stance is deliberately shared with WL-15: secrets stay in files, the smallest
principal performs the operation, and the transport is not asked to provide application
authorization. WL-15's broker TLS and credential policy remain its own design.

## 4. Threats and mitigations

### T1. Host impersonation or man-in-the-middle

**Threat.** An attacker intercepts or redirects SSH traffic to a different host and receives the
debug token or project sources.

**Mitigations.** The SSH client runs with `BatchMode=yes` and `StrictHostKeyChecking=yes`; an
unknown host key fails instead of prompting, and the test suite asserts those options. The
development host's `known_hosts` and SSH agent are the only credential sources. The driver never
disables host-key checking and never accepts a password on standard input.

**Residual risk.** A user who adds an attacker's key to `known_hosts`, or an attacker with the
user's SSH agent, is outside this boundary.

### T2. Credential or secret disclosure

**Threat.** The debug token or control nonce leaks through argv, logs, JSON output, error
messages, or the session descriptor.

**Mitigations.** The control nonce is created as a `0600` file and passed with
`--control-nonce-file`, not argv. The descriptor omits the nonce and token. The endpoint file
remains user-only and is read over SSH only after ownership and mode checks. Serve and launch
diagnostics print paths, build IDs, and generations, never tokens. Error tails come from the
serve log, which does not include the endpoint token.

**Residual risk.** A local user who can read the development host's process memory or the remote
account's files can read the secrets; SSH key protection is an operating-system responsibility.

### T3. Unauthorized debug or control access from another local user

**Threat.** Another account on the remote host connects to the debug socket or control socket.

**Mitigations.** Both endpoints are loopback-only and user-only. The debug endpoint file and audit
log are `0600`; the control socket is `0600`; the nonce file is `0600`. The descriptor and
endpoint are read through a script that checks `stat` ownership and that group/other bits are
absent. The driver rejects a descriptor or endpoint path outside the work directory and a
non-loopback debug endpoint.

**Residual risk.** Root or the same login user can always reach the endpoints. The model does not
attempt to isolate users inside one account.

### T4. Command injection through launch configuration

**Threat.** A hostile or mistyped `remote`, `remoteDir`, `remoteStc`, `remoteSession`, or project
path injects shell syntax into SSH or SFTP commands.

**Mitigations.** The SSH destination accepts only `[user@]host` with ASCII letters, digits, `.`,
`_`, and `-`; a leading `-` is rejected. Remote paths must be absolute or `~/`-relative, and
empty, `.`, `..`, backslash, and control characters are rejected. Commands are built as argv
vectors and each argument is single-quoted; only a NUL character is rejected as unrepresentable.
SFTP batches quote paths and the mirror accepts a restricted character set. Build and deploy
paths are constructed from validated pieces, not from free-form text.

**Residual risk.** A project file name outside the mirror character set is rejected with
`STC-E-REMOTE-005` rather than uploaded; the user must rename it. This is deliberate.

### T5. Path traversal and symlink escape

**Threat.** A descriptor or configured path points outside the remote work directory, or a
symlink redirects a write into another directory.

**Mitigations.** `stc-remote/1` validation requires every descriptor path to be absolute and
inside the work directory. The driver rejects `/` and the login home as work directories. The
source mirror is reset only at `<work>/src`; candidate and build outputs stay under the work
directory. Private-file reads require a regular file, the login user's ownership, and no group or
other permission bits. The SFTP mirror layout is derived from package roots, not from untrusted
relative strings.

**Residual risk.** The login user can replace its own files with symlinks; the mirror is written
with `put`, so a pre-existing symlink in the work directory is the user's own trust domain. The
work directory must be dedicated to `stc`.

### T6. Remote command and control abuse

**Threat.** A compromised or malicious remote `stc` performs actions beyond the user's intent, or
the local driver sends control operations without authorization.

**Mitigations.** The remote `stc --version` must match the local driver before a session, and the
serve descriptor's `stc_version` is checked again. Control operations use the existing nonce-
authenticated channel and its fixed operation allowlist. The driver invokes only `serve`,
`build`, and `--version`; it does not run arbitrary remote shell commands from configuration.
SSH runs in batch mode, so no interactive prompt can be answered by the adapter.

**Residual risk.** The remote account controls its own `stc` binary and can modify its runtime.
Version matching detects accidental drift, not a deliberate replacement by the account owner.

### T7. Tunnel or SSH failure mid-session

**Threat.** The SSH process exits, the network drops, or the remote host stops; the editor keeps
displaying stale values.

**Mitigations.** The local debug client observes connection loss and ends the DAP session with a
distinct summary. The launch-owned `RemoteHost` stops the remote serve through control at
teardown; an attached serve is left running. The tunnel client uses `ExitOnForwardFailure=yes`
and SSH keepalives. There is no automatic reconnect, so stale state is never silently replayed.

**Residual risk.** An attached host can keep running after the editor disappears, by design. A
user who reattaches starts a new session and must re-apply breakpoints.

### T8. Denial of service and resource exhaustion

**Threat.** A remote or local process stalls the build, the tunnel, or a command; the editor hangs
or the remote host fills its disk.

**Mitigations.** Command, upload, and tunnel setup have deadlines; the SSH client uses
`ConnectTimeout=10` and `ServerAliveInterval=15`/`ServerAliveCountMax=3`. The descriptor and
endpoint reads cap size. The source mirror and candidate directories are reset per operation.
The descriptor is written atomically with `0600`.

**Residual risk.** The remote build may legitimately take longer than a fixed deadline; the
driver reports `STC-E-REMOTE-006` and leaves the host for the operator to inspect. Disk usage is
bounded only by the user's work-directory policy.

### T9. State or artifact mismatch after deployment

**Threat.** The editor debugs a build that does not match local sources, or a cold restart uses
stale state.

**Mitigations.** Every launch mirrors sources, uploads them, verifies SHA-256 per file, builds
remotely, and compares the GOT production source fingerprint with the local fingerprint. Attach
compares the descriptor fingerprint; drift requires an explicit `remoteDeploy` choice. Online
deployment uses `compare_manifests`; an incompatible plan is reported as cold-required. A stable
state file under the work directory carries retained state across cold restarts, with the
runtime's existing compatibility policy.

**Residual risk.** Build IDs are host-specific and are reported rather than recomputed locally.
The source fingerprint does not cover compiler or manifest settings; a changed `[build]` setting
is detected by the build result and the manifest comparison, not by the source hash alone.

## 5. Secret-handling rules

- Never pass the debug token or control nonce on a command line or in an environment variable
  that is visible in a process list.
- Never print either value in human or JSON diagnostics; report paths and hashes instead.
- Create descriptor, nonce, endpoint, and audit files with mode `0600` and verify ownership and
  mode before reading them over SSH.
- Use the SSH agent and `known_hosts`; do not add password prompts, `StrictHostKeyChecking=no`,
  or host-key auto-accept to any code path.
- Do not copy remote credentials, private keys, or host access notes into the repository.

## 6. Verification mapping

| Control | Test or evidence |
| --- | --- |
| SSH batch mode and strict host keys | `remote::tests::ssh_arguments_enforce_batch_mode_and_known_hosts` |
| SSH authentication failure is non-interactive | `remote_launch` Linux fake-SSH auth-failure test |
| Non-loopback endpoint rejected | `remote::tests::tunnel_arguments_forward_only_a_loopback_socket` |
| Descriptor/endpoint permissions | `remote_launch` Linux permission tests; `remote::tests` private-file tests |
| Path traversal and unsafe characters | `remote::tests::remote_paths_are_absolute_or_home_relative_and_stay_safe`, mirror-path tests |
| Command injection | `remote::tests::parses_safe_ssh_targets_and_rejects_option_injection`, quoting tests |
| Version mismatch | `remote_launch` version test; descriptor version check |
| Unauthorized control | Existing `stc-runtime` control-channel tests (nonce and operation allowlist) |
| Tunnel drop | Negative/reattach evidence in [Remote-Debug-Runbook.md](Remote-Debug-Runbook.md) |
| Source/artifact mismatch | `remote::tests::source_fingerprints_must_match`, `remote_status_cli` drift test |

The live negative and reattachment checks are recorded in the runbook during HIL verification.
