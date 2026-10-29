#!/usr/bin/env python3
"""WL-4 RT campaign harness.

Runs the prebuilt rt-bench launcher on the RT host, drives the local control
channel, samples /proc, and writes a stable summary. The harness is installed
root-owned at /home/r2k/stc-rt/tools/rt/campaign.py and is started through the
scoped ``stc-wl4-run campaign`` wrapper (R-2); it never stores credentials and
never prints the control nonce.

Usage:
    python3 campaign.py --regime r0 --duration 10m --cycle-time 1ms \
        --out /home/r2k/stc-rt/results/2026-10-29-r0
    python3 campaign.py --mode online --online-case all --duration 5s \
        --out /home/r2k/stc-rt/results/2026-10-29-online
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import math
import os
import platform
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
from array import array
from datetime import datetime, timezone
from pathlib import Path

CONTROL_PROTOCOL = 1
METRICS_PROTOCOL = "stc-rt-metrics/1"
SUMMARY_SCHEMA = "stc-rt-summary/1"
HOST_SCHEMA = "stc-rt-host/1"
CONTROL_LOG_SCHEMA = "stc-rt-control/1"

CASES = ("code-only", "field-add-remove", "incompatible", "trapping")
LOAD_PROFILES = ("none", "cpu", "mixed")
REGIMES = ("r0", "r1", "r2")
WORKLOADS = ("base", "light", "heavy")
CONTROL_MODES = ("socket", "none")

DEFAULT_BUNDLE = "/home/r2k/stc-rt/rt-bench"
DEFAULT_RT_CPU = 3
DEFAULT_RT_PRIORITY = 90
DEFAULT_WARMUP = "2s"
DEFAULT_DURATION = "10s"

DURATION_FACTORS = {
    "ns": 1,
    "us": 1_000,
    "µs": 1_000,
    "ms": 1_000_000,
    "s": 1_000_000_000,
    "m": 60_000_000_000,
    "h": 3_600_000_000_000,
}
DURATION_RE = re.compile(r"^([0-9]+)(ns|us|µs|ms|s|m|h)?$")


class HarnessError(RuntimeError):
    """A run could not be completed as specified."""


def parse_duration_ns(text: str) -> int:
    """Parses a duration literal (``10ms``, ``5s``, ``1m``) into nanoseconds."""
    match = DURATION_RE.match(str(text).strip())
    if match is None:
        raise ValueError(f"invalid duration {text!r}")
    number = int(match.group(1))
    unit = match.group(2) or "s"
    return number * DURATION_FACTORS[unit]


def format_ns(value: int | float | None) -> str:
    """Formats nanoseconds for a human-readable summary."""
    if value is None:
        return "null"
    value = float(value)
    sign = "-" if value < 0 else ""
    value = abs(value)
    if value < 1_000:
        return f"{sign}{value:.0f} ns"
    if value < 1_000_000:
        return f"{sign}{value / 1_000:.3f} us"
    if value < 1_000_000_000:
        return f"{sign}{value / 1_000_000:.3f} ms"
    return f"{sign}{value / 1_000_000_000:.3f} s"


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def steady_cycle_count(warmup_ns: int, duration_ns: int, cycle_time_ns: int) -> int:
    """Returns the launcher cycle count for a control-disabled steady run.

    The control-disabled leg cannot ask the host for its cycle counter, so it
    runs a fixed number of cycles covering the warm-up and measurement window,
    plus one cycle of margin.
    """
    return math.ceil((warmup_ns + duration_ns) / cycle_time_ns) + 1


def nearest_rank(values: array | list[int], percent: float) -> int | None:
    """Returns the nearest-rank percentile of an integer sample."""
    if not values:
        return None
    ordered = sorted(values)
    index = math.ceil(percent / 100.0 * len(ordered)) - 1
    index = max(0, min(index, len(ordered) - 1))
    return int(ordered[index])


def stats(values: array | list[int]) -> dict:
    """Returns the stable percentile block used by the summary schema."""
    if not values:
        return {
            "count": 0,
            "min": None,
            "p50": None,
            "p90": None,
            "p99": None,
            "p99_9": None,
            "max": None,
        }
    ordered = sorted(values)
    return {
        "count": len(ordered),
        "min": int(ordered[0]),
        "p50": nearest_rank(ordered, 50),
        "p90": nearest_rank(ordered, 90),
        "p99": nearest_rank(ordered, 99),
        "p99_9": nearest_rank(ordered, 99.9),
        "max": int(ordered[-1]),
    }


def read_text(path: str | Path) -> str | None:
    try:
        return Path(path).read_text(encoding="utf-8", errors="replace").strip()
    except OSError:
        return None


def run_text(command: list[str], timeout: float = 5.0) -> str | None:
    try:
        completed = subprocess.run(
            command,
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    text = completed.stdout.strip()
    return text or None


def sha256_file(path: str | Path) -> str | None:
    path = Path(path)
    if not path.is_file():
        return None
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def write_json(path: str | Path, value: object) -> None:
    Path(path).write_text(
        json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


def collect_host_info() -> dict:
    """Collects the host conditions recorded with every run."""
    governors: dict[str, str | None] = {}
    for cpu_dir in sorted(Path("/sys/devices/system/cpu").glob("cpu[0-9]*")):
        name = cpu_dir.name
        governors[name] = read_text(cpu_dir / "cpufreq" / "scaling_governor")
    idle_states: list[dict] = []
    idle_root = Path("/sys/devices/system/cpu/cpu0/cpuidle")
    if idle_root.is_dir():
        for state in sorted(idle_root.glob("state[0-9]*")):
            idle_states.append(
                {
                    "name": read_text(state / "name"),
                    "latency_us": read_text(state / "latency"),
                    "residency_us": read_text(state / "residency"),
                }
            )
    return {
        "schema": HOST_SCHEMA,
        "collected_at": utc_now(),
        "hostname": socket.gethostname(),
        "uname": {
            "system": platform.system(),
            "release": platform.release(),
            "version": platform.version(),
            "machine": platform.machine(),
        },
        "os_release": read_text("/etc/os-release"),
        "cpu_count": os.cpu_count(),
        "cmdline": read_text("/proc/cmdline"),
        "isolated": read_text("/sys/devices/system/cpu/isolated") or "",
        "nohz_full": read_text("/sys/devices/system/cpu/nohz_full") or "",
        "online": read_text("/sys/devices/system/cpu/online"),
        "governors": governors,
        "idle_states": idle_states,
        "clocksource": read_text(
            "/sys/devices/system/clocksource/clocksource0/current_clocksource"
        ),
        "preempt": read_text("/sys/kernel/debug/sched/preempt"),
        "sched_rt_runtime_us": read_text("/proc/sys/kernel/sched_rt_runtime_us"),
        "perf_event_paranoid": read_text("/proc/sys/kernel/perf_event_paranoid"),
        "swap": read_text("/proc/swaps"),
        "services_running": run_text(
            [
                "systemctl",
                "list-units",
                "--type=service",
                "--state=running",
                "--no-legend",
            ]
        ),
        "dmi": {
            "sys_vendor": read_text("/sys/class/dmi/id/sys_vendor"),
            "product_name": read_text("/sys/class/dmi/id/product_name"),
            "board_name": read_text("/sys/class/dmi/id/board_name"),
            "bios_version": read_text("/sys/class/dmi/id/bios_version"),
            "bios_date": read_text("/sys/class/dmi/id/bios_date"),
        },
        "uptime": read_text("/proc/uptime"),
        "available_tracers": read_text("/sys/kernel/tracing/available_tracers"),
    }


def control_body(
    nonce: str,
    request_id: int,
    op: str,
    module: str | Path | None = None,
    manifest: str | Path | None = None,
) -> dict:
    """Builds one control request body (protocol version 1)."""
    body = {
        "protocol": CONTROL_PROTOCOL,
        "nonce": nonce,
        "id": request_id,
        "op": op,
    }
    if module is not None:
        body["module"] = str(module)
    if manifest is not None:
        body["manifest"] = str(manifest)
    return body


def redact_request(body: dict) -> dict:
    """Returns a log-safe copy of a control request."""
    return {key: value for key, value in body.items() if key != "nonce"}


def validate_control_response(response: object, expected_id: int) -> dict:
    """Validates the common control response fields."""
    if not isinstance(response, dict):
        raise HarnessError("the control response is not an object")
    if response.get("protocol") != CONTROL_PROTOCOL:
        raise HarnessError(
            f"unexpected control protocol {response.get('protocol')!r} "
            f"(expected {CONTROL_PROTOCOL})"
        )
    if response.get("id") != expected_id:
        raise HarnessError(
            f"control response id {response.get('id')!r} does not match {expected_id}"
        )
    return response


class ControlClient:
    """Newline-delimited JSON control client for the local runtime socket."""

    def __init__(self, endpoint: str, nonce: str, timeout: float = 15.0):
        self.endpoint = endpoint
        self.nonce = nonce
        self.next_id = 1
        self.socket: socket.socket | None = None
        self.reader = None
        deadline = time.monotonic() + timeout
        last_error: Exception | None = None
        while time.monotonic() < deadline:
            try:
                client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                client.settimeout(2.0)
                client.connect(endpoint)
                self.socket = client
                self.reader = client.makefile("rb")
                return
            except OSError as error:
                last_error = error
                try:
                    client.close()
                except (OSError, UnboundLocalError):
                    pass
                time.sleep(0.02)
        raise HarnessError(
            f"cannot connect to control endpoint {endpoint!r}: {last_error}"
        )

    def request(
        self,
        op: str,
        module: str | Path | None = None,
        manifest: str | Path | None = None,
        timeout: float = 10.0,
    ) -> dict:
        if self.socket is None or self.reader is None:
            raise HarnessError("control client is not connected")
        request_id = self.next_id
        self.next_id += 1
        body = control_body(self.nonce, request_id, op, module, manifest)
        line = json.dumps(body, separators=(",", ":")) + "\n"
        self.socket.settimeout(timeout)
        self.socket.sendall(line.encode("utf-8"))
        raw = self.reader.readline()
        if not raw:
            raise HarnessError("the control host closed the connection")
        try:
            response = json.loads(raw.decode("utf-8"))
        except json.JSONDecodeError as error:
            raise HarnessError(f"invalid control response: {error}") from error
        return validate_control_response(response, request_id)

    def close(self) -> None:
        try:
            if self.reader is not None:
                self.reader.close()
        except OSError:
            pass
        try:
            if self.socket is not None:
                self.socket.close()
        except OSError:
            pass


class ControlLog:
    """Writes redacted control requests and responses."""

    def __init__(self, path: Path):
        self.handle = path.open("w", encoding="utf-8")
        self.handle.write(
            json.dumps({"schema": CONTROL_LOG_SCHEMA}) + "\n"
        )

    def record(
        self,
        direction: str,
        body: dict | None = None,
        response: dict | None = None,
        t_ns: int | None = None,
    ) -> None:
        entry: dict = {
            "t_ns": time.monotonic_ns() if t_ns is None else t_ns,
            "direction": direction,
        }
        if body is not None:
            entry["request"] = redact_request(body)
        if response is not None:
            entry["response"] = response
        self.handle.write(json.dumps(entry, separators=(",", ":")) + "\n")
        self.handle.flush()

    def close(self) -> None:
        self.handle.close()


def find_single(directory: Path, pattern: str, description: str) -> Path:
    matches = sorted(directory.glob(pattern))
    if len(matches) != 1:
        raise HarnessError(
            f"expected exactly one {description} matching {pattern!r} in {directory}, "
            f"found {len(matches)}"
        )
    return matches[0]


def workload_directory(bundle: Path, workload: str) -> Path:
    if workload == "base":
        return bundle
    return bundle / "workloads" / workload


def resolve_workload(bundle: Path, workload: str) -> dict:
    directory = workload_directory(bundle, workload)
    launcher = directory / "rt-bench"
    if not launcher.is_file():
        raise HarnessError(f"launcher not found: {launcher}")
    module = find_single(directory, "rt-bench-*.so", "launcher module")
    manifest = directory / "rt-bench.stc-got.json"
    if not manifest.is_file():
        raise HarnessError(f"manifest not found: {manifest}")
    return {
        "workload": workload,
        "directory": directory,
        "launcher": launcher,
        "module": module,
        "manifest": manifest,
    }


def manifest_build_id(path: Path) -> str | None:
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    body = document.get("body") if isinstance(document, dict) else None
    if isinstance(body, dict):
        for container in (body.get("header"), body):
            if isinstance(container, dict):
                build_id = container.get("build_id")
                if isinstance(build_id, str):
                    return build_id
    return None


def resolve_candidate(bundle: Path, case: str) -> dict:
    directory = bundle / "candidates" / case
    module = find_single(directory, "rt-bench-*.so", f"{case} module")
    manifest = directory / "rt-bench.stc-got.json"
    manifest_bin = directory / "rt-bench.stc-got.bin"
    if not manifest.is_file():
        raise HarnessError(f"candidate manifest not found: {manifest}")
    if not manifest_bin.is_file():
        raise HarnessError(f"candidate binary manifest not found: {manifest_bin}")
    return {
        "case": case,
        "directory": directory,
        "module": module,
        "manifest": manifest,
        "manifest_bin": manifest_bin,
        "build_id": manifest_build_id(manifest),
    }


def parse_metrics(path: Path) -> dict:
    """Reads the stc-rt-metrics/1 JSONL stream."""
    header = None
    summary = None
    cycles: list[dict] = []
    with path.open("r", encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, start=1):
            line = line.strip()
            if not line:
                continue
            try:
                record = json.loads(line)
            except json.JSONDecodeError as error:
                raise HarnessError(
                    f"invalid metrics JSON at {path}:{line_number}: {error}"
                ) from error
            if not isinstance(record, dict):
                raise HarnessError(f"metrics record at {path}:{line_number} is not an object")
            kind = record.get("record")
            if kind == "header":
                if record.get("protocol") != METRICS_PROTOCOL:
                    raise HarnessError(
                        f"unexpected metrics protocol {record.get('protocol')!r} "
                        f"(expected {METRICS_PROTOCOL})"
                    )
                header = record
            elif kind == "cycle":
                cycles.append(record)
            elif kind == "summary":
                summary = record
            else:
                raise HarnessError(
                    f"unknown metrics record {kind!r} at {path}:{line_number}"
                )
    if header is None:
        raise HarnessError(f"metrics file {path} has no header record")
    return {"header": header, "cycles": cycles, "summary": summary}


def _first_sample(records: list[dict], field_one: str, field_two: str) -> tuple | None:
    for record in records:
        first = record.get(field_one)
        second = record.get(field_two)
        if first is not None and second is not None:
            return (int(first), int(second))
    return None


def _last_sample(records: list[dict], field_one: str, field_two: str) -> tuple | None:
    for record in reversed(records):
        first = record.get(field_one)
        second = record.get(field_two)
        if first is not None and second is not None:
            return (int(first), int(second))
    return None


def metrics_stats(
    parsed: dict,
    measure_start_cycle: int,
    duration_ns: int,
    cycle_time_ns: int,
) -> dict:
    """Computes the stable metrics block for the measurement window."""
    all_cycles: list[dict] = parsed["cycles"]
    measured = [record for record in all_cycles if int(record["cycle"]) >= measure_start_cycle]
    wake_lateness = array("q", (int(record.get("wake_lateness_ns", 0)) for record in measured))
    execution = array("q", (int(record.get("execution_ns", 0)) for record in measured))
    scan_overrun = array("q", (int(record.get("scan_overrun_ns", 0)) for record in measured))
    missed = sum(1 for record in measured if record.get("missed_deadline"))
    skipped = sum(int(record.get("skipped_periods", 0)) for record in measured)
    faults_first = _first_sample(measured, "minor_faults", "major_faults")
    faults_last = _last_sample(measured, "minor_faults", "major_faults")
    alloc_first = _first_sample(measured, "allocations", "frees")
    alloc_last = _last_sample(measured, "allocations", "frees")
    return {
        "cycles_in_file": int(all_cycles[-1]["cycle"]) if all_cycles else 0,
        "measured_cycles": len(measured),
        "measure_start_cycle": measure_start_cycle,
        "expected_cycles": duration_ns // cycle_time_ns if cycle_time_ns else None,
        "missed_deadlines": missed,
        "skipped_periods": skipped,
        "wake_lateness_ns": stats(wake_lateness),
        "execution_ns": stats(execution),
        "scan_overrun_ns": {
            "max": int(max(scan_overrun)) if scan_overrun else None,
            "p99": nearest_rank(scan_overrun, 99),
        },
        "minor_faults": {
            "first": faults_first[0] if faults_first else None,
            "last": faults_last[0] if faults_last else None,
            "delta": (
                faults_last[0] - faults_first[0] if faults_first and faults_last else None
            ),
        },
        "major_faults": {
            "first": faults_first[1] if faults_first else None,
            "last": faults_last[1] if faults_last else None,
            "delta": (
                faults_last[1] - faults_first[1] if faults_first and faults_last else None
            ),
        },
        "allocations": {
            "first": alloc_first[0] if alloc_first else None,
            "last": alloc_last[0] if alloc_last else None,
            "delta": (
                alloc_last[0] - alloc_first[0] if alloc_first and alloc_last else None
            ),
        },
        "frees": {
            "first": alloc_first[1] if alloc_first else None,
            "last": alloc_last[1] if alloc_last else None,
            "delta": (
                alloc_last[1] - alloc_first[1] if alloc_first and alloc_last else None
            ),
        },
        "launcher_summary": parsed["summary"],
    }


def generation_transition(cycles: list[dict], generation: int) -> dict:
    """Finds the first and last cycle of a deployed generation."""
    matching = [int(record["cycle"]) for record in cycles if record.get("generation") == generation]
    if not matching:
        return {
            "activation_cycle": None,
            "rollback_cycle": None,
            "cycles_in_generation": 0,
        }
    last = matching[-1]
    rollback = None
    for record in cycles:
        cycle = int(record["cycle"])
        if cycle > last and record.get("generation") != generation:
            rollback = cycle
            break
    return {
        "activation_cycle": matching[0],
        "last_cycle": last,
        "rollback_cycle": rollback,
        "cycles_in_generation": len(matching),
    }


def read_proc_sample(pid: int) -> dict | None:
    """Reads one /proc sample for the launcher process."""
    stat_path = Path(f"/proc/{pid}/stat")
    status_path = Path(f"/proc/{pid}/status")
    try:
        stat_text = stat_path.read_text(encoding="utf-8")
    except OSError:
        return None
    close = stat_text.rfind(")")
    if close < 0:
        return None
    fields = stat_text[close + 1 :].split()
    if len(fields) < 22:
        return None
    sample: dict = {
        "t_ns": time.monotonic_ns(),
        "pid": pid,
        "state": fields[0],
        "minflt": int(fields[7]),
        "majflt": int(fields[9]),
        "utime_ticks": int(fields[11]),
        "stime_ticks": int(fields[12]),
        "num_threads": int(fields[17]),
        "rss_pages": int(fields[21]),
    }
    try:
        status_text = status_path.read_text(encoding="utf-8")
    except OSError:
        return sample
    status: dict = {}
    for line in status_text.splitlines():
        if ":" not in line:
            continue
        key, value = line.split(":", 1)
        status[key.strip()] = value.strip()
    sample["vm_rss_kb"] = _parse_int_prefix(status.get("VmRSS"))
    sample["threads"] = _parse_int_prefix(status.get("Threads"))
    sample["voluntary_ctxt_switches"] = _parse_int_prefix(
        status.get("voluntary_ctxt_switches")
    )
    sample["nonvoluntary_ctxt_switches"] = _parse_int_prefix(
        status.get("nonvoluntary_ctxt_switches")
    )
    return sample


def _parse_int_prefix(value: str | None) -> int | None:
    if not value:
        return None
    token = value.split()[0]
    try:
        return int(token)
    except ValueError:
        return None


class ProcSampler(threading.Thread):
    """Writes /proc samples for one process at a fixed rate."""

    def __init__(self, pid: int, path: Path, hz: float = 1.0):
        super().__init__(name="stc-rt-proc", daemon=True)
        self.pid = pid
        self.path = path
        self.hz = max(hz, 0.1)
        self.stop_event = threading.Event()
        self.samples = 0
        self.error: str | None = None

    def run(self) -> None:
        interval = 1.0 / self.hz
        try:
            with self.path.open("w", encoding="utf-8") as handle:
                while not self.stop_event.wait(interval):
                    sample = read_proc_sample(self.pid)
                    if sample is None:
                        continue
                    handle.write(json.dumps(sample, separators=(",", ":")) + "\n")
                    handle.flush()
                    self.samples += 1
        except OSError as error:
            self.error = str(error)

    def stop(self) -> None:
        self.stop_event.set()
        self.join(timeout=5.0)


def load_command(
    profile: str,
    cpus: list[int],
    timeout_s: int,
    temp_path: Path | None = None,
) -> list[str] | None:
    """Builds the stress-ng load command for a load profile."""
    if profile == "none":
        return None
    if not cpus:
        raise HarnessError("a load profile needs at least one CPU")
    workers = max(1, len(cpus))
    command = [
        "/usr/bin/taskset",
        "-c",
        ",".join(str(cpu) for cpu in cpus),
        "/usr/bin/stress-ng",
        "--timeout",
        f"{max(timeout_s, 1)}s",
        "--metrics-brief",
        "--cpu",
        str(workers * 2),
        "--cpu-method",
        "matrix",
    ]
    if profile == "mixed":
        command += [
            "--vm",
            str(workers),
            "--vm-bytes",
            "256M",
            "--hdd",
            str(workers),
            "--hdd-bytes",
            "128M",
            "--sock",
            str(workers),
            "--sock-ops",
            "1000000",
        ]
        if temp_path is not None:
            command += ["--temp-path", str(temp_path)]
    return command


class LoadRunner:
    """Starts and stops the external load generator."""

    def __init__(
        self,
        profile: str,
        cpus: list[int],
        duration_ns: int,
        log_path: Path,
    ):
        self.profile = profile
        self.cpus = cpus
        self.duration_ns = duration_ns
        self.log_path = log_path
        self.temp_path = log_path.parent / "load-tmp"
        self.command = load_command(
            profile,
            cpus,
            int(duration_ns / 1_000_000_000) + 30,
            self.temp_path,
        )
        self.process: subprocess.Popen | None = None
        self.handle = None
        self.exit_code: int | None = None

    def start(self) -> None:
        if self.command is None:
            return
        self.temp_path.mkdir(parents=True, exist_ok=True)
        self.handle = self.log_path.open("w", encoding="utf-8")
        self.process = subprocess.Popen(
            self.command,
            cwd=str(self.temp_path),
            stdout=self.handle,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )

    def stop(self) -> None:
        if self.process is not None and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        if self.process is not None:
            self.exit_code = self.process.returncode
        if self.handle is not None:
            self.handle.close()
        shutil.rmtree(self.temp_path, ignore_errors=True)


def launcher_command(
    workload: dict,
    cycle_time_ns: int,
    rt_priority: int,
    rt_cpu: int,
    metrics_path: Path,
    endpoint: str,
    nonce: str,
    control_mode: str = "socket",
    cycles: int = 0,
) -> list[str]:
    command = [
        str(workload["launcher"]),
        "--cycles",
        str(cycles),
        "--cycle-time",
        f"{cycle_time_ns}ns",
    ]
    if control_mode == "socket":
        command += ["--control", endpoint, "--control-nonce", nonce]
    elif control_mode != "none":
        raise HarnessError(f"unknown control mode {control_mode!r}")
    command += [
        "--rt",
        "--rt-priority",
        str(rt_priority),
        "--rt-cpu",
        str(rt_cpu),
        "--rt-metrics",
        str(metrics_path),
        "--status",
    ]
    return command


def redacted_command(command: list[str], nonce: str) -> list[str]:
    return ["<redacted>" if item == nonce else item for item in command]


class Launcher:
    """One running launcher process and its control client."""

    def __init__(
        self,
        workload: dict,
        cycle_time_ns: int,
        rt_priority: int,
        rt_cpu: int,
        run_dir: Path,
        bundle: Path,
        control_mode: str = "socket",
        cycles: int = 0,
    ):
        self.workload = workload
        self.cycle_time_ns = cycle_time_ns
        self.rt_priority = rt_priority
        self.rt_cpu = rt_cpu
        self.run_dir = run_dir
        self.bundle = bundle
        self.control_mode = control_mode
        self.cycles = cycles
        self.nonce = os.urandom(16).hex() if control_mode == "socket" else ""
        self.endpoint_dir = (
            Path(tempfile.mkdtemp(prefix="stc-rt-"))
            if control_mode == "socket"
            else None
        )
        self.endpoint = (
            str(self.endpoint_dir / "control.sock") if self.endpoint_dir else ""
        )
        self.metrics_path = run_dir / "cycles.jsonl"
        self.log_path = run_dir / "launcher.log"
        self.command = launcher_command(
            workload,
            cycle_time_ns,
            rt_priority,
            rt_cpu,
            self.metrics_path,
            self.endpoint,
            self.nonce,
            control_mode,
            cycles,
        )
        self.process: subprocess.Popen | None = None
        self.client: ControlClient | None = None
        self.log_handle = None
        self.clean_stop = False

    def start(self) -> None:
        self.log_handle = self.log_path.open("w", encoding="utf-8")
        environment = os.environ.copy()
        runtime = self.bundle / "runtime"
        library_path = environment.get("LD_LIBRARY_PATH", "")
        environment["LD_LIBRARY_PATH"] = (
            f"{runtime}:{library_path}" if library_path else str(runtime)
        )
        self.process = subprocess.Popen(
            self.command,
            cwd=str(self.workload["directory"]),
            env=environment,
            stdout=self.log_handle,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        if self.control_mode == "none":
            return
        try:
            self.client = ControlClient(self.endpoint, self.nonce, timeout=15.0)
        except HarnessError:
            self._stop_process()
            tail = self.log_tail()
            raise HarnessError(
                f"launcher did not open its control endpoint; log tail:\n{tail}"
            )

    def status(self) -> dict:
        if self.client is None:
            raise HarnessError("launcher control client is not connected")
        return self.client.request("status")

    def deploy(self, module: Path, manifest: Path, timeout: float = 30.0) -> dict:
        if self.client is None:
            raise HarnessError("launcher control client is not connected")
        return self.client.request("deploy", module, manifest, timeout=timeout)

    def stop(self) -> None:
        if self.client is not None:
            try:
                self.client.request("stop", timeout=10.0)
                self.clean_stop = True
            except HarnessError:
                pass
        if self.process is not None:
            try:
                self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self._stop_process()
            if self.control_mode == "none":
                self.clean_stop = self.process.returncode == 0
        self._close()

    def _stop_process(self) -> None:
        if self.process is None:
            return
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()

    def _close(self) -> None:
        if self.client is not None:
            self.client.close()
            self.client = None
        if self.log_handle is not None:
            self.log_handle.close()
            self.log_handle = None
        if self.endpoint_dir is not None:
            shutil.rmtree(self.endpoint_dir, ignore_errors=True)

    def log_tail(self, lines: int = 20) -> str:
        try:
            text = self.log_path.read_text(encoding="utf-8", errors="replace").splitlines()
        except OSError:
            return ""
        return "\n".join(text[-lines:])

    def return_code(self) -> int | None:
        return None if self.process is None else self.process.returncode


def gzip_and_hash(source: Path) -> tuple[Path, str | None]:
    """Gzips a JSONL file and returns the archive and its SHA256."""
    target = source.with_suffix(source.suffix + ".gz")
    with source.open("rb") as reader, gzip.open(target, "wb", compresslevel=9) as writer:
        shutil.copyfileobj(reader, writer)
    source.unlink()
    return target, sha256_file(target)


def proc_summary(path: Path) -> dict:
    samples: list[dict] = []
    if path.is_file():
        with path.open("r", encoding="utf-8") as handle:
            for line in handle:
                line = line.strip()
                if line:
                    try:
                        samples.append(json.loads(line))
                    except json.JSONDecodeError:
                        continue
    if not samples:
        return {"samples": 0}
    first = samples[0]
    last = samples[-1]
    rss = [sample.get("vm_rss_kb") for sample in samples if sample.get("vm_rss_kb") is not None]
    return {
        "samples": len(samples),
        "first": first,
        "last": last,
        "rss_kb_max": max(rss) if rss else None,
        "minflt_delta": _delta(first.get("minflt"), last.get("minflt")),
        "majflt_delta": _delta(first.get("majflt"), last.get("majflt")),
        "utime_ticks_delta": _delta(first.get("utime_ticks"), last.get("utime_ticks")),
        "stime_ticks_delta": _delta(first.get("stime_ticks"), last.get("stime_ticks")),
        "context_switches_delta": _delta(
            first.get("voluntary_ctxt_switches"), last.get("voluntary_ctxt_switches")
        ),
        "nonvoluntary_switches_delta": _delta(
            first.get("nonvoluntary_ctxt_switches"),
            last.get("nonvoluntary_ctxt_switches"),
        ),
    }


def _delta(first: object, last: object) -> int | None:
    if first is None or last is None:
        return None
    try:
        return int(last) - int(first)
    except (TypeError, ValueError):
        return None


def write_host_file(out: Path, host: dict) -> None:
    write_json(out / "host.json", host)


def write_summary_files(out: Path, summary: dict) -> None:
    write_json(out / "summary.json", summary)
    (out / "summary.md").write_text(render_markdown(summary), encoding="utf-8")
    for path in out.iterdir():
        if path.is_file():
            try:
                os.chmod(path, 0o644)
            except OSError:
                pass
    os.chmod(out, 0o755)


def summary_metrics_block(
    parsed: dict,
    measure_start_cycle: int,
    duration_ns: int,
    cycle_time_ns: int,
) -> dict:
    block = metrics_stats(parsed, measure_start_cycle, duration_ns, cycle_time_ns)
    block["header"] = parsed["header"]
    return block


def steady_run(args: argparse.Namespace, bundle: Path, host: dict) -> dict:
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    write_host_file(out, host)
    workload = resolve_workload(bundle, args.workload)
    cycle_time_ns = parse_duration_ns(args.cycle_time)
    duration_ns = parse_duration_ns(args.duration)
    warmup_ns = parse_duration_ns(args.warmup)
    control_mode = args.control_mode
    cycles = (
        steady_cycle_count(warmup_ns, duration_ns, cycle_time_ns)
        if control_mode == "none"
        else 0
    )
    launcher = Launcher(
        workload,
        cycle_time_ns,
        args.rt_priority,
        args.rt_cpu,
        out,
        bundle,
        control_mode,
        cycles,
    )
    load = LoadRunner(
        args.load_profile,
        _load_cpus(args.regime, args.load_profile, args.rt_cpu),
        warmup_ns + duration_ns,
        out / "load.log",
    )
    sampler: ProcSampler | None = None
    control_log = (
        ControlLog(out / "control.jsonl") if control_mode == "socket" else None
    )
    errors: list[str] = []
    status_start: dict | None = None
    status_measure: dict | None = None
    status_end: dict | None = None
    measure_start_cycle = 0
    started_at = utc_now()
    try:
        launcher.start()
        if control_log is not None:
            status_start = launcher.status()
            control_log.record("status", response=status_start)
        load.start()
        if warmup_ns > 0:
            time.sleep(warmup_ns / 1_000_000_000)
        if control_log is not None:
            status_measure = launcher.status()
            control_log.record("status", response=status_measure)
            measure_start_cycle = int(status_measure.get("cycles", 0))
        else:
            # Without a control channel the cycle counter is not available;
            # approximate the warm-up boundary from the configured period.
            measure_start_cycle = math.ceil(warmup_ns / cycle_time_ns)
        if launcher.process is not None:
            sampler = ProcSampler(launcher.process.pid, out / "proc.jsonl")
            sampler.start()
        if control_log is not None:
            _wait_measured(args, launcher, control_log, duration_ns)
            status_end = launcher.status()
            control_log.record("status", response=status_end)
        else:
            time.sleep(duration_ns / 1_000_000_000)
        launcher.stop()
        if control_mode == "none" and launcher.return_code() not in (0, None):
            errors.append(f"launcher exited with status {launcher.return_code()}")
    except (HarnessError, OSError, ValueError) as error:
        errors.append(str(error))
        launcher.stop()
    finally:
        load.stop()
        if sampler is not None:
            sampler.stop()
        if control_log is not None:
            control_log.close()

    parsed = None
    if launcher.metrics_path.is_file():
        try:
            parsed = parse_metrics(launcher.metrics_path)
        except HarnessError as error:
            errors.append(str(error))
    else:
        errors.append(f"metrics file missing: {launcher.metrics_path}")

    proc_path = out / "proc.jsonl"
    proc_block = proc_summary(proc_path)
    cycles_gz, cycles_hash = (None, None)
    proc_gz, proc_hash = (None, None)
    if launcher.metrics_path.is_file():
        cycles_gz, cycles_hash = gzip_and_hash(launcher.metrics_path)
    if proc_path.is_file():
        proc_gz, proc_hash = gzip_and_hash(proc_path)

    metrics_block = None
    if parsed is not None:
        metrics_block = summary_metrics_block(
            parsed, measure_start_cycle, duration_ns, cycle_time_ns
        )
        if measure_start_cycle == 0 and metrics_block["measured_cycles"] == 0:
            errors.append("the measurement window contains no cycles")

    summary = {
        "schema": SUMMARY_SCHEMA,
        "run_id": out.name,
        "mode": "steady",
        "regime": args.regime,
        "workload": args.workload,
        "cycle_time_ns": cycle_time_ns,
        "duration_ns": duration_ns,
        "warmup_ns": warmup_ns,
        "rt": {"priority": args.rt_priority, "cpu": args.rt_cpu},
        "load_profile": args.load_profile,
        "control_mode": control_mode,
        "status_hz": args.status_hz,
        "started_at": started_at,
        "finished_at": utc_now(),
        "host": host,
        "bundle": {
            "path": str(bundle),
            "workload": workload["workload"],
            "launcher": str(workload["launcher"]),
            "module": str(workload["module"]),
            "manifest": str(workload["manifest"]),
            "build_id": manifest_build_id(workload["manifest"]),
        },
        "launcher": {
            "command": redacted_command(launcher.command, launcher.nonce),
            "pid": launcher.process.pid if launcher.process else None,
            "return_code": launcher.return_code(),
            "clean_stop": launcher.clean_stop,
            "log_tail": launcher.log_tail() if errors else None,
        },
        "load": {
            "profile": args.load_profile,
            "cpus": _load_cpus(args.regime, args.load_profile, args.rt_cpu),
            "command": load.command,
            "pid": load.process.pid if load.process else None,
            "exit_code": load.exit_code,
        },
        "status": {
            "start": status_start,
            "measure_start": status_measure,
            "end": status_end,
        },
        "metrics": metrics_block,
        "proc": proc_block,
        "files": {
            "cycles": str(cycles_gz) if cycles_gz else None,
            "cycles_sha256": cycles_hash,
            "proc": str(proc_gz) if proc_gz else None,
            "proc_sha256": proc_hash,
            "launcher_log": str(launcher.log_path),
            "launcher_log_sha256": sha256_file(launcher.log_path),
            "control_log": (
                str(out / "control.jsonl") if control_log is not None else None
            ),
            "control_log_sha256": (
                sha256_file(out / "control.jsonl") if control_log is not None else None
            ),
            "host": str(out / "host.json"),
        },
        "result": "failed" if errors else "passed",
        "errors": errors,
    }
    write_summary_files(out, summary)
    return summary


def _wait_measured(
    args: argparse.Namespace,
    launcher: Launcher,
    control_log: ControlLog,
    duration_ns: int,
) -> None:
    duration_s = duration_ns / 1_000_000_000
    if args.status_hz <= 0:
        time.sleep(duration_s)
        return
    interval = 1.0 / args.status_hz
    deadline = time.monotonic() + duration_s
    while time.monotonic() < deadline:
        response = launcher.status()
        control_log.record("status", response=response)
        time.sleep(min(interval, max(0.0, deadline - time.monotonic())))


def _load_cpus(regime: str, profile: str, cycle_cpu: int) -> list[int]:
    if profile == "none":
        return []
    total = os.cpu_count() or 1
    others = [cpu for cpu in range(total) if cpu != cycle_cpu]
    if regime == "r2":
        return [cycle_cpu]
    return others


def online_run(args: argparse.Namespace, bundle: Path, host: dict) -> dict:
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    write_host_file(out, host)
    cases = CASES if args.online_case == "all" else (args.online_case,)
    results: dict[str, dict] = {}
    errors: list[str] = []
    for case in cases:
        try:
            results[case] = online_case(args, bundle, host, out / case, case)
            if results[case]["result"] != "passed":
                errors.append(f"{case}: {', '.join(results[case]['errors']) or 'failed'}")
        except (HarnessError, OSError, ValueError) as error:
            errors.append(f"{case}: {error}")
            results[case] = {
                "schema": SUMMARY_SCHEMA,
                "case": case,
                "mode": "online",
                "result": "failed",
                "errors": [str(error)],
            }
            case_dir = out / case
            case_dir.mkdir(parents=True, exist_ok=True)
            write_summary_files(case_dir, results[case])
    summary = {
        "schema": SUMMARY_SCHEMA,
        "run_id": out.name,
        "mode": "online",
        "regime": args.regime,
        "cycle_time_ns": parse_duration_ns(args.cycle_time),
        "duration_ns": parse_duration_ns(args.duration),
        "warmup_ns": parse_duration_ns(args.warmup),
        "rt": {"priority": args.rt_priority, "cpu": args.rt_cpu},
        "load_profile": args.load_profile,
        "started_at": utc_now(),
        "finished_at": utc_now(),
        "host": host,
        "bundle": str(bundle),
        "cases": results,
        "result": "failed" if errors else "passed",
        "errors": errors,
    }
    write_summary_files(out, summary)
    return summary


def online_case(
    args: argparse.Namespace,
    bundle: Path,
    host: dict,
    out: Path,
    case: str,
) -> dict:
    out.mkdir(parents=True, exist_ok=True)
    write_host_file(out, host)
    workload = resolve_workload(bundle, args.workload)
    candidate = resolve_candidate(bundle, case)
    cycle_time_ns = parse_duration_ns(args.cycle_time)
    duration_ns = parse_duration_ns(args.duration)
    warmup_ns = parse_duration_ns(args.warmup)
    launcher = Launcher(
        workload, cycle_time_ns, args.rt_priority, args.rt_cpu, out, bundle
    )
    load = LoadRunner(
        args.load_profile,
        _load_cpus(args.regime, args.load_profile, args.rt_cpu),
        warmup_ns + duration_ns + 30_000_000_000,
        out / "load.log",
    )
    sampler: ProcSampler | None = None
    control_log = ControlLog(out / "control.jsonl")
    errors: list[str] = []
    status_start: dict | None = None
    status_after: dict | None = None
    status_final: dict | None = None
    base_build: str | None = None
    deploy_response: dict | None = None
    request_ns = 0
    response_ns = 0
    rollback: dict | None = None
    started_at = utc_now()
    try:
        launcher.start()
        status_start = launcher.status()
        control_log.record("status", response=status_start)
        base_build = status_start.get("active_build")
        load.start()
        if warmup_ns > 0:
            time.sleep(warmup_ns / 1_000_000_000)
        if launcher.process is not None:
            sampler = ProcSampler(launcher.process.pid, out / "proc.jsonl")
            sampler.start()
        request_ns = time.monotonic_ns()
        deploy_response = launcher.deploy(
            candidate["module"], candidate["manifest_bin"]
        )
        response_ns = time.monotonic_ns()
        control_log.record(
            "deploy", response=deploy_response, t_ns=response_ns
        )
        status_after = launcher.status()
        control_log.record("status", response=status_after)
        if case in ("code-only", "field-add-remove"):
            if not deploy_response.get("ok") or not deploy_response.get("activated"):
                errors.append(f"expected activation, got {deploy_response}")
            elif status_after.get("active_build") != candidate["build_id"]:
                errors.append(
                    "active build after deploy is "
                    f"{status_after.get('active_build')!r}, expected {candidate['build_id']!r}"
                )
        elif case == "incompatible":
            if deploy_response.get("ok"):
                errors.append("incompatible candidate unexpectedly activated")
            if not deploy_response.get("cold_required"):
                errors.append(
                    "incompatible candidate did not report cold_required: "
                    f"{deploy_response}"
                )
            if status_after.get("active_build") != base_build:
                errors.append("incompatible deploy changed the active build")
        elif case == "trapping":
            if not deploy_response.get("ok") or not deploy_response.get("activated"):
                errors.append(f"trapping candidate was not activated: {deploy_response}")
            else:
                rollback = _wait_rollback(launcher, control_log, base_build, errors)
                status_final = launcher.status()
                control_log.record("status", response=status_final)
        if duration_ns > 0:
            time.sleep(duration_ns / 1_000_000_000)
        status_final = launcher.status()
        control_log.record("status", response=status_final)
        launcher.stop()
    except (HarnessError, OSError, ValueError) as error:
        errors.append(str(error))
        launcher.stop()
    finally:
        load.stop()
        if sampler is not None:
            sampler.stop()
        control_log.close()

    parsed = None
    if launcher.metrics_path.is_file():
        try:
            parsed = parse_metrics(launcher.metrics_path)
        except HarnessError as error:
            errors.append(str(error))
    else:
        errors.append(f"metrics file missing: {launcher.metrics_path}")

    proc_path = out / "proc.jsonl"
    proc_block = proc_summary(proc_path)
    cycles_gz, cycles_hash = (None, None)
    proc_gz, proc_hash = (None, None)
    if launcher.metrics_path.is_file():
        cycles_gz, cycles_hash = gzip_and_hash(launcher.metrics_path)
    if proc_path.is_file():
        proc_gz, proc_hash = gzip_and_hash(proc_path)

    transition = None
    if parsed is not None and deploy_response is not None:
        generation = deploy_response.get("generation")
        if isinstance(generation, int):
            transition = generation_transition(parsed["cycles"], generation)
    metrics_block = None
    if parsed is not None:
        metrics_block = summary_metrics_block(parsed, 0, duration_ns, cycle_time_ns)

    summary = {
        "schema": SUMMARY_SCHEMA,
        "run_id": out.name,
        "case": case,
        "mode": "online",
        "regime": args.regime,
        "workload": args.workload,
        "cycle_time_ns": cycle_time_ns,
        "duration_ns": duration_ns,
        "warmup_ns": warmup_ns,
        "rt": {"priority": args.rt_priority, "cpu": args.rt_cpu},
        "load_profile": args.load_profile,
        "started_at": started_at,
        "finished_at": utc_now(),
        "host": host,
        "bundle": {
            "path": str(bundle),
            "workload": workload["workload"],
            "build_id": manifest_build_id(workload["manifest"]),
        },
        "candidate": {
            "case": case,
            "module": str(candidate["module"]),
            "manifest": str(candidate["manifest"]),
            "manifest_bin": str(candidate["manifest_bin"]),
            "build_id": candidate["build_id"],
            "base_build_id": base_build,
        },
        "deploy": {
            "request_ns": request_ns,
            "response_ns": response_ns,
            "response_latency_ns": response_ns - request_ns if response_ns else None,
            "response": deploy_response,
            "generation_transition": transition,
            "rollback": rollback,
            "status_after": status_after,
            "status_final": status_final,
        },
        "launcher": {
            "command": redacted_command(launcher.command, launcher.nonce),
            "pid": launcher.process.pid if launcher.process else None,
            "return_code": launcher.return_code(),
            "clean_stop": launcher.clean_stop,
            "log_tail": launcher.log_tail() if errors else None,
        },
        "load": {
            "profile": args.load_profile,
            "cpus": _load_cpus(args.regime, args.load_profile, args.rt_cpu),
            "command": load.command,
            "pid": load.process.pid if load.process else None,
            "exit_code": load.exit_code,
        },
        "metrics": metrics_block,
        "proc": proc_block,
        "files": {
            "cycles": str(cycles_gz) if cycles_gz else None,
            "cycles_sha256": cycles_hash,
            "proc": str(proc_gz) if proc_gz else None,
            "proc_sha256": proc_hash,
            "launcher_log": str(launcher.log_path),
            "launcher_log_sha256": sha256_file(launcher.log_path),
            "control_log": str(out / "control.jsonl"),
            "control_log_sha256": sha256_file(out / "control.jsonl"),
            "host": str(out / "host.json"),
        },
        "result": "failed" if errors else "passed",
        "errors": errors,
    }
    write_summary_files(out, summary)
    return summary


def _wait_rollback(
    launcher: Launcher,
    control_log: ControlLog,
    base_build: str | None,
    errors: list[str],
) -> dict:
    started_ns = time.monotonic_ns()
    deadline = time.monotonic() + 5.0
    polls = 0
    last: dict | None = None
    while time.monotonic() < deadline:
        last = launcher.status()
        control_log.record("status", response=last)
        polls += 1
        if last.get("active_build") == base_build:
            return {
                "observed": True,
                "polls": polls,
                "elapsed_ns": time.monotonic_ns() - started_ns,
                "status": last,
            }
        time.sleep(0.005)
    errors.append(
        "trapping candidate did not roll back to the base generation within 5s"
    )
    return {
        "observed": False,
        "polls": polls,
        "elapsed_ns": time.monotonic_ns() - started_ns,
        "status": last,
    }


def render_markdown(summary: dict) -> str:
    lines = [f"# RT campaign {summary.get('run_id', '')}", ""]
    lines.append(f"- Schema: `{summary.get('schema')}`")
    lines.append(f"- Mode: `{summary.get('mode')}`")
    lines.append(f"- Regime: `{summary.get('regime')}`")
    lines.append(f"- Result: **{summary.get('result')}**")
    lines.append("")
    if summary.get("mode") == "steady":
        metrics = summary.get("metrics") or {}
        lines += [
            "## Run",
            "",
            f"- Workload: `{summary.get('workload')}`",
            f"- Cycle time: {format_ns(summary.get('cycle_time_ns'))}",
            f"- Duration: {format_ns(summary.get('duration_ns'))}",
            f"- Warm-up: {format_ns(summary.get('warmup_ns'))}",
            f"- RT priority/CPU: {summary.get('rt')}",
            f"- Load profile: `{summary.get('load_profile')}`",
            f"- Control: `{summary.get('control_mode', 'socket')}` "
            f"(status {summary.get('status_hz', 0)} Hz)",
            f"- Build: `{(summary.get('bundle') or {}).get('build_id')}`",
            "",
            "## Metrics",
            "",
            "| Metric | min | p50 | p90 | p99 | p99.9 | max |",
            "| --- | --- | --- | --- | --- | --- | --- |",
        ]
        for label, key in (("Wake lateness", "wake_lateness_ns"), ("Execution", "execution_ns")):
            block = metrics.get(key) or {}
            lines.append(
                f"| {label} | {format_ns(block.get('min'))} | {format_ns(block.get('p50'))} "
                f"| {format_ns(block.get('p90'))} | {format_ns(block.get('p99'))} "
                f"| {format_ns(block.get('p99_9'))} | {format_ns(block.get('max'))} |"
            )
        lines += [
            "",
            f"- Measured cycles: {metrics.get('measured_cycles')} "
            f"(expected {metrics.get('expected_cycles')})",
            f"- Missed deadlines: {metrics.get('missed_deadlines')}",
            f"- Skipped periods: {metrics.get('skipped_periods')}",
            f"- Minor/major fault delta: "
            f"{(metrics.get('minor_faults') or {}).get('delta')} / "
            f"{(metrics.get('major_faults') or {}).get('delta')}",
            f"- Allocation/free delta: "
            f"{(metrics.get('allocations') or {}).get('delta')} / "
            f"{(metrics.get('frees') or {}).get('delta')}",
            "",
        ]
    elif summary.get("mode") == "online":
        lines += [
            "## Online-change cases",
            "",
            "| Case | Base build | Candidate build | Result | Cold required | Rollback |",
            "| --- | --- | --- | --- | --- | --- |",
        ]
        for case, result in (summary.get("cases") or {}).items():
            deploy = result.get("deploy") or {}
            response = deploy.get("response") or {}
            candidate = result.get("candidate") or {}
            rollback = deploy.get("rollback") or {}
            lines.append(
                f"| {case} | `{candidate.get('base_build_id')}` "
                f"| `{candidate.get('build_id')}` | {result.get('result')} "
                f"| {bool(response.get('cold_required'))} "
                f"| {bool(rollback.get('observed'))} |"
            )
        lines.append("")
    if summary.get("errors"):
        lines += ["## Errors", ""]
        lines += [f"- {error}" for error in summary["errors"]]
        lines.append("")
    return "\n".join(lines)


def validate_args(args: argparse.Namespace) -> None:
    if args.rt_priority < 1 or args.rt_priority > 99:
        raise HarnessError("--rt-priority must be in 1..=99")
    if args.rt_cpu < 0 or args.rt_cpu > 63:
        raise HarnessError("--rt-cpu must be in 0..=63")
    if args.control_mode not in CONTROL_MODES:
        raise HarnessError(f"unknown control mode {args.control_mode!r}")
    if args.mode == "online" and args.control_mode != "socket":
        raise HarnessError("online mode needs the control socket")
    if args.control_mode == "none" and args.status_hz != 0:
        raise HarnessError("--status-hz must be 0 when control is disabled")
    if parse_duration_ns(args.cycle_time) <= 0:
        raise HarnessError("--cycle-time must be positive")
    if parse_duration_ns(args.duration) <= 0:
        raise HarnessError("--duration must be positive")
    if parse_duration_ns(args.warmup) < 0:
        raise HarnessError("--warmup must not be negative")
    if args.mode == "online" and args.online_case not in CASES + ("all",):
        raise HarnessError(f"unknown online case {args.online_case!r}")
    if args.status_hz < 0:
        raise HarnessError("--status-hz must not be negative")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="WL-4 RT campaign harness (stc-rt-summary/1)"
    )
    parser.add_argument("--mode", choices=("steady", "online"), default="steady")
    parser.add_argument("--regime", choices=REGIMES, default="r0")
    parser.add_argument("--duration", default=DEFAULT_DURATION)
    parser.add_argument("--cycle-time", default="1ms")
    parser.add_argument("--warmup", default=DEFAULT_WARMUP)
    parser.add_argument("--workload", choices=WORKLOADS, default="base")
    parser.add_argument("--rt-cpu", type=int, default=DEFAULT_RT_CPU)
    parser.add_argument("--rt-priority", type=int, default=DEFAULT_RT_PRIORITY)
    parser.add_argument("--bundle", default=DEFAULT_BUNDLE)
    parser.add_argument("--load-profile", choices=LOAD_PROFILES, default=None)
    parser.add_argument("--status-hz", type=float, default=0.0)
    parser.add_argument("--control-mode", choices=CONTROL_MODES, default="socket")
    parser.add_argument("--online-case", choices=CASES + ("all",), default="all")
    parser.add_argument("--out", required=True)
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        validate_args(args)
    except (HarnessError, ValueError) as error:
        parser.error(str(error))
    if args.load_profile is None:
        args.load_profile = "none" if args.regime == "r0" else "mixed"
    get_euid = getattr(os, "geteuid", None)
    if get_euid is None or get_euid() != 0:
        print(
            "campaign.py must run as root through "
            "`sudo -n /usr/local/sbin/stc-wl4-run campaign`",
            file=sys.stderr,
        )
        return 2
    bundle = Path(args.bundle).resolve()
    if not bundle.is_dir():
        print(f"bundle directory not found: {bundle}", file=sys.stderr)
        return 2
    out = Path(args.out).resolve()
    if out.exists() and any(out.iterdir()):
        print(f"output directory is not empty: {out}", file=sys.stderr)
        return 2
    host = collect_host_info()
    if args.mode == "steady":
        summary = steady_run(args, bundle, host)
    else:
        summary = online_run(args, bundle, host)
    write_json(out / "host.json", host)
    print(json.dumps({"result": summary["result"], "out": str(out)}, sort_keys=True))
    return 0 if summary["result"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
