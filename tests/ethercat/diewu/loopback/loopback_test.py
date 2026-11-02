#!/usr/bin/env python3
"""All-points loopback test for the DIEWU 16I/16O fixture (WL-5.2 phase 6).

The script launches the `diewu-loopback` project through `stc debug` launch
mode, which builds and runs it with the packaged compiler/runtime. It reads
the test result through the same `var.read` debug surface used for the phase-4
status evidence: it polls DIEWU-LOOPBACK::DONE, then reads PASS, FAILMASK,
OBSERVEDMASK, and the status-device mirror, prints the 16-channel matrix, and
exits 0 only when every channel passed and the fieldbus stayed healthy.

Exit codes:
  0  all 16 channels passed and the fieldbus status is healthy
  1  at least one channel failed, or the fieldbus reported a fault
  2  harness, build, debug-transport, or timeout error

The script does not use root, `--rt`, or a locally rebuilt stc. The committed
phase-6 procedure always runs it with the default `--io ethercat`; the
`--io memory` option exists only for an offline smoke test of the script and
is not phase-6 evidence.
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import subprocess
import sys
import threading
import time

PACKAGE = "DIEWU-LOOPBACK"
CHANNELS = 16
SHIM_ENV = "STC_ETHERCAT_LIBRARY"


class HarnessError(Exception):
    """A build, debug-transport, or test-timeout failure (exit 2)."""


class DebugSession:
    """One `stc debug` launch session with a JSON-lines command channel."""

    def __init__(self, stc: str, manifest: str, io_backend: str, cycle_time: str,
                 startup_timeout: float) -> None:
        command = [
            stc,
            "debug",
            "--manifest-path",
            manifest,
            "--io",
            io_backend,
            "--cycles",
            "0",
            "--cycle-time",
            cycle_time,
            "--message-format",
            "json",
        ]
        try:
            self.proc = subprocess.Popen(
                command,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=None,
                text=True,
                bufsize=1,
            )
        except OSError as error:
            raise HarnessError(f"cannot start {stc!r}: {error}") from error
        self._lines: "queue.Queue[object]" = queue.Queue()
        self._eof = object()
        self._reader = threading.Thread(target=self._read_stdout, daemon=True)
        self._reader.start()
        self.session_result = self._wait_for_session(startup_timeout)

    def _read_stdout(self) -> None:
        assert self.proc.stdout is not None
        try:
            for line in self.proc.stdout:
                self._lines.put(line)
        finally:
            self._lines.put(self._eof)

    def _next_message(self, timeout: float) -> dict:
        if timeout <= 0:
            raise HarnessError("timed out waiting for the debug session")
        try:
            item = self._lines.get(timeout=timeout)
        except queue.Empty as error:
            raise HarnessError(
                "timed out waiting for a response from the debug session"
            ) from error
        if item is self._eof:
            code = self.proc.poll()
            raise HarnessError(
                f"`stc debug` exited (status {code}) before the expected response; "
                "see the build/runtime output above"
            )
        assert isinstance(item, str)
        try:
            return json.loads(item)
        except json.JSONDecodeError as error:
            raise HarnessError(
                f"non-JSON output on the debug channel: {item.rstrip()!r}"
            ) from error

    def _response(self, timeout: float) -> dict:
        deadline = time.monotonic() + timeout
        while True:
            message = self._next_message(deadline - time.monotonic())
            kind = message.get("kind")
            if kind == "event":
                if message.get("event") == "session-ended":
                    reason = message.get("reason", "unknown")
                    raise HarnessError(f"the debug session ended ({reason})")
                continue
            if kind == "response":
                if message.get("status") != "ok":
                    error = message.get("error") or {}
                    code = error.get("code", "STC-E-DEBUG-003")
                    detail = error.get("message", "the debug request failed")
                    raise HarnessError(f"{code}: {detail}")
                result = message.get("result")
                return result if isinstance(result, dict) else {}
            # Build diagnostics are plain JSON objects without a message kind.
            print(f"stc: {json.dumps(message, separators=(',', ':'))}",
                  file=sys.stderr)

    def _wait_for_session(self, timeout: float) -> dict:
        deadline = time.monotonic() + timeout
        result = self._response(max(deadline - time.monotonic(), 0.0))
        if not result.get("session"):
            raise HarnessError("the debug session did not start")
        return result

    def _write(self, text: str) -> None:
        if self.proc.poll() is not None:
            raise HarnessError("`stc debug` exited before the command was sent")
        assert self.proc.stdin is not None
        try:
            self.proc.stdin.write(text)
            self.proc.stdin.flush()
        except (BrokenPipeError, OSError) as error:
            raise HarnessError(f"cannot write to `stc debug`: {error}") from error

    def read_raw(self, key: str, timeout: float = 15.0) -> tuple[str, object]:
        self._write(f"print {PACKAGE}::{key.upper()}\n")
        result = self._response(timeout)
        value = result.get("value")
        if not isinstance(value, dict) or "type" not in value or "value" not in value:
            raise HarnessError(f"var.read {key} returned no typed value")
        return str(value["type"]), value["value"]

    def read_bool(self, key: str, timeout: float = 15.0) -> bool:
        kind, value = self.read_raw(key, timeout)
        if kind != "BOOL" or not isinstance(value, bool):
            raise HarnessError(f"var.read {key} returned {kind} {value!r}, expected BOOL")
        return value

    def read_int(self, key: str, timeout: float = 15.0) -> int:
        kind, value = self.read_raw(key, timeout)
        if isinstance(value, bool) or not isinstance(value, str):
            raise HarnessError(f"var.read {key} returned {kind} {value!r}, expected an integer")
        try:
            return int(value, 10)
        except ValueError as error:
            raise HarnessError(
                f"var.read {key} returned {kind} {value!r}, expected a decimal integer"
            ) from error

    def close(self) -> None:
        if self.proc.poll() is None:
            try:
                self._write("quit\n")
            except HarnessError:
                pass
            try:
                self.proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.proc.terminate()
                try:
                    self.proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    self.proc.kill()
                    self.proc.wait()
        if self.proc.stdin is not None and not self.proc.stdin.closed:
            try:
                self.proc.stdin.close()
            except OSError:
                pass


def parse_args(argv: list[str]) -> argparse.Namespace:
    here = os.path.dirname(os.path.abspath(__file__))
    parser = argparse.ArgumentParser(
        description="Run the DIEWU 16I/16O loopback all-points test."
    )
    parser.add_argument("--stc", default="stc", help="packaged stc binary (default: stc)")
    parser.add_argument(
        "--manifest",
        default=os.path.join(here, "stc.toml"),
        help="loopback manifest (default: stc.toml next to this script)",
    )
    parser.add_argument(
        "--io",
        default="ethercat",
        help="I/O backend (default: ethercat; memory is a local smoke test only)",
    )
    parser.add_argument(
        "--cycle-time",
        default="1ms",
        help="scan and EtherCAT cycle time (default: 1ms)",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=60.0,
        help="seconds to wait for Done after the session starts (default: 60)",
    )
    parser.add_argument(
        "--startup-timeout",
        type=float,
        default=180.0,
        help="seconds to wait for the build and debug session (default: 180)",
    )
    parser.add_argument(
        "--poll-interval",
        type=float,
        default=0.2,
        help="seconds between Done polls (default: 0.2)",
    )
    return parser.parse_args(argv)


def check_environment(args: argparse.Namespace) -> None:
    if not os.path.isfile(args.manifest):
        raise HarnessError(f"manifest not found: {args.manifest}")
    if args.io == "ethercat":
        shim = os.environ.get(SHIM_ENV)
        if not shim:
            raise HarnessError(
                f"{SHIM_ENV} is not set; export the CI-built libstc_io_ethercat.so"
            )
        if not os.path.isfile(shim):
            raise HarnessError(f"{SHIM_ENV} does not name a file: {shim}")
    else:
        print(
            f"warning: --io {args.io} is a local smoke test, not phase-6 evidence",
            file=sys.stderr,
        )


def wait_for_done(session: DebugSession, timeout: float, poll_interval: float) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if session.read_bool("DONE"):
            return
        time.sleep(poll_interval)
    raise HarnessError(
        f"the all-points test did not set Done within {timeout:.0f}s; "
        "check that the slave reached OP and that the run is healthy"
    )


def print_matrix(fail_mask: int, observed_mask: int) -> None:
    print()
    print("channel  output  input  expected  observed  result")
    print("-------  ------  -----  --------  --------  ------")
    for channel in range(CHANNELS):
        bit = 1 << channel
        observed = "1" if observed_mask & bit else "0"
        result = "FAIL" if fail_mask & bit else "PASS"
        print(
            f"{channel:>7}  OUT{channel + 1:<3}  IN{channel + 1:<4} "
            f"1         {observed:<9} {result}"
        )
    print()


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    session = None
    try:
        check_environment(args)
        session = DebugSession(
            args.stc, args.manifest, args.io, args.cycle_time, args.startup_timeout
        )
        build = session.session_result.get("build_id", "unknown")
        generation = session.session_result.get("generation", "unknown")
        print(f"loopback: manifest={args.manifest}")
        print(f"loopback: io={args.io} cycles=0 cycle-time={args.cycle_time}")
        print(f"loopback: debug session build={build} generation={generation}")

        wait_for_done(session, args.timeout, args.poll_interval)
        passed = session.read_bool("PASS")
        fail_mask = session.read_int("FAILMASK")
        observed_mask = session.read_int("OBSERVEDMASK")
        output_mask = session.read_int("OUTPUTMASK")
        channel = session.read_int("CHANNEL")
        scans = session.read_int("SCANCOUNT")
        faults = session.read_int("STATUSFAULTS")
        stale = session.read_bool("STATUSINPUTSTALE")
        recoveries = session.read_int("STATUSRECOVERIES")
        wc_state = session.read_int("STATUSWCSTATE")
        working_counter = session.read_int("STATUSWORKINGCOUNTER")
        link_up = session.read_bool("STATUSLINKUP")
        master_phase = session.read_int("STATUSPHASE")
        slave_al = session.read_int("STATUSSLAVEALSTATE")
        slave_online = session.read_bool("STATUSSLAVEONLINE")
        slave_operational = session.read_bool("STATUSSLAVEOPERATIONAL")
        exchanges = session.read_int("STATUSEXCHANGES")

        print_matrix(fail_mask, observed_mask)
        print(f"Pass:          {passed}")
        print(f"FailMask:      0x{fail_mask:04X}")
        print(f"ObservedMask:  0x{observed_mask:04X}")
        print(f"OutputMask:    0x{output_mask:04X} (outputs are off after the walk)")
        print(f"Channel:       {channel} (16 means every channel was sampled)")
        print(f"Scans:         {scans}")
        print(
            f"Status:        faults={faults} stale={str(stale).lower()} "
            f"recoveries={recoveries} wc_state={wc_state} wc={working_counter} "
            f"link_up={str(link_up).lower()} master_phase={master_phase} "
            f"slave_al_state={slave_al} slave_online={str(slave_online).lower()} "
            f"slave_operational={str(slave_operational).lower()} exchanges={exchanges}"
        )

        problems: list[str] = []
        if not passed:
            problems.append("Pass is false")
        if fail_mask != 0:
            problems.append(f"FailMask is 0x{fail_mask:04X}")
        if faults != 0:
            problems.append(f"fieldbus faults={faults}")
        if stale:
            problems.append("input.stale is true")
        if wc_state != 2:
            problems.append(f"domain.wc_state={wc_state}, expected 2")
        if not link_up:
            problems.append("master.link_up is false")
        if not slave_operational:
            problems.append("slave is not operational")
        if channel != CHANNELS:
            problems.append(f"Channel={channel}, expected {CHANNELS}")
        if problems:
            print(f"Acceptance:    FAIL ({'; '.join(problems)})")
            return 1
        print("Acceptance:    PASS (16/16 channels; fieldbus healthy)")
        return 0
    except HarnessError as error:
        print(f"loopback_test: error: {error}", file=sys.stderr)
        return 2
    except KeyboardInterrupt:
        print("loopback_test: interrupted", file=sys.stderr)
        return 2
    finally:
        if session is not None:
            session.close()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
