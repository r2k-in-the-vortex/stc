#!/usr/bin/env python3
"""Compare the default run loop with ``--rt`` on Windows and macOS.

The script runs a prebuilt launcher directly (no control socket, no root),
discards a fixed warm-up window, and writes a stable JSON summary plus a
Markdown table. It is portable across Windows, macOS, and Linux and uses only
the Python 3 standard library.

Usage:
    python tools/rt/sched_compare.py --launcher build/rt-bench \
        --cycle-time 1ms --cycles 20000 --runs 5 --mode baseline \
        --out results/baseline
    python tools/rt/sched_compare.py --launcher build/rt-bench \
        --cycle-time 1ms --cycles 20000 --runs 5 --mode rt \
        --out results/rt

The ``rt`` mode passes ``--rt --rt-metrics <file>`` to the launcher and
analyses the ``stc-rt-metrics/1`` JSONL stream. The ``baseline`` mode runs the
default loop; because the metrics stream is opt-in with ``--rt`` (D-249), it
has no per-cycle records. The baseline is therefore compared by wall-clock
cycle time, while the ``rt`` run supplies the execution and wake-lateness
percentiles. This limitation is recorded in the summary and in
``KNOWN-LIMITATIONS.md``.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import re
import socket
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

SUMMARY_SCHEMA = "stc-sched-compare/1"
HOST_SCHEMA = "stc-sched-host/1"
METRICS_PROTOCOL = "stc-rt-metrics/1"
MODES = ("baseline", "rt")
DEFAULT_CYCLES = 20_000
DEFAULT_RUNS = 5
DEFAULT_CYCLE_TIME = "1ms"
DEFAULT_WARMUP_CYCLES = 2_000

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


class CompareError(RuntimeError):
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
    """Formats nanoseconds for a human-readable Markdown cell."""
    if value is None:
        return "n/a"
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


def nearest_rank(values: list[int], percent: float) -> int | None:
    """Returns the nearest-rank percentile of an integer sample."""
    if not values:
        return None
    ordered = sorted(values)
    index = math.ceil(percent / 100.0 * len(ordered)) - 1
    index = max(0, min(index, len(ordered) - 1))
    return int(ordered[index])


def stats(values: list[int]) -> dict:
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


def command_output(command: list[str], timeout: float = 10.0) -> str | None:
    """Runs a short host-information command and returns its first line."""
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
    for line in completed.stdout.splitlines():
        line = line.strip()
        if line:
            return line
    return None


def read_text(path: str | Path) -> str | None:
    try:
        return Path(path).read_text(encoding="utf-8", errors="replace").strip()
    except OSError:
        return None


def sha256_file(path: str | Path) -> str | None:
    path = Path(path)
    if not path.is_file():
        return None
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def cpu_model() -> str | None:
    """Returns the host CPU model using only standard-library commands."""
    system = platform.system()
    if system == "Windows":
        return command_output(
            [
                "powershell",
                "-NoProfile",
                "-Command",
                "(Get-CimInstance Win32_Processor | Select-Object -First 1 "
                "-ExpandProperty Name)",
            ]
        ) or platform.processor() or None
    if system == "Darwin":
        return command_output(["sysctl", "-n", "machdep.cpu.brand_string"]) or None
    text = read_text("/proc/cpuinfo")
    if text:
        for line in text.splitlines():
            if line.lower().startswith("model name"):
                _, _, value = line.partition(":")
                value = value.strip()
                if value:
                    return value
    return platform.processor() or None


def os_product() -> dict:
    """Returns the OS product details used in the host table."""
    system = platform.system()
    if system == "Darwin":
        return {
            "product_version": command_output(["sw_vers", "-productVersion"]),
            "build_version": command_output(["sw_vers", "-buildVersion"]),
        }
    if system == "Windows":
        release, version, service_pack, _ = platform.win32_ver()
        return {
            "product_version": release,
            "build_version": version,
            "service_pack": service_pack or None,
        }
    return {
        "os_release": read_text("/etc/os-release"),
    }


def collect_host_info() -> dict:
    """Collects host, OS, and CPU metadata recorded with every run."""
    uname = platform.uname()
    runner = {}
    for key in (
        "RUNNER_OS",
        "RUNNER_ARCH",
        "ImageOS",
        "ImageVersion",
        "GITHUB_RUN_ID",
        "GITHUB_WORKFLOW",
    ):
        value = os.environ.get(key)
        if value:
            runner[key] = value
    return {
        "schema": HOST_SCHEMA,
        "collected_at": utc_now(),
        "hostname": socket.gethostname(),
        "platform": platform.platform(),
        "system": uname.system,
        "release": uname.release,
        "version": uname.version,
        "machine": uname.machine,
        "processor": uname.processor or platform.processor() or None,
        "cpu_model": cpu_model(),
        "cpu_count": os.cpu_count(),
        "python": platform.python_version(),
        "os_product": os_product(),
        "runner": runner,
    }


def manifest_build_id(launcher: Path) -> tuple[str | None, str | None]:
    """Returns ``(build_id, manifest_path)`` for the launcher's sibling manifest."""
    candidates = sorted(launcher.parent.glob("*.stc-got.json"))
    if len(candidates) != 1:
        return None, None
    manifest = candidates[0]
    try:
        document = json.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None, str(manifest)
    body = document.get("body") if isinstance(document, dict) else None
    if isinstance(body, dict):
        for container in (body.get("header"), body):
            if isinstance(container, dict):
                build_id = container.get("build_id")
                if isinstance(build_id, str):
                    return build_id, str(manifest)
    return None, str(manifest)


def launcher_command(
    launcher: Path,
    cycle_time_ns: int,
    cycles: int,
    mode: str,
    metrics_path: Path | None,
) -> list[str]:
    """Builds the launcher command for one mode."""
    if mode not in MODES:
        raise CompareError(f"unknown mode {mode!r}")
    command = [
        str(launcher),
        "--cycles",
        str(cycles),
        "--cycle-time",
        f"{cycle_time_ns}ns",
    ]
    if mode == "rt":
        if metrics_path is None:
            raise CompareError("rt mode needs a metrics path")
        command += ["--rt", "--rt-metrics", str(metrics_path)]
    return command


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
                raise CompareError(
                    f"invalid metrics JSON at {path}:{line_number}: {error}"
                ) from error
            if not isinstance(record, dict):
                raise CompareError(
                    f"metrics record at {path}:{line_number} is not an object"
                )
            kind = record.get("record")
            if kind == "header":
                if record.get("protocol") != METRICS_PROTOCOL:
                    raise CompareError(
                        f"unexpected metrics protocol {record.get('protocol')!r} "
                        f"(expected {METRICS_PROTOCOL})"
                    )
                header = record
            elif kind == "cycle":
                cycles.append(record)
            elif kind == "summary":
                summary = record
            else:
                raise CompareError(
                    f"unknown metrics record {kind!r} at {path}:{line_number}"
                )
    if header is None:
        raise CompareError(f"metrics file {path} has no header record")
    return {"header": header, "cycles": cycles, "summary": summary}


def analyze_metrics(parsed: dict, warmup_cycles: int) -> dict:
    """Computes the measured-window statistics for one metrics stream."""
    cycles = parsed["cycles"]
    measured = [record for record in cycles if int(record["cycle"]) > warmup_cycles]
    if not measured:
        raise CompareError(
            f"the warm-up window ({warmup_cycles} cycles) leaves no measured cycles"
        )
    execution = [int(record.get("execution_ns", 0)) for record in measured]
    wake_lateness = [int(record.get("wake_lateness_ns", 0)) for record in measured]
    missed = sum(1 for record in measured if record.get("missed_deadline"))
    skipped = sum(int(record.get("skipped_periods", 0)) for record in measured)
    summary = parsed.get("summary")
    dropped = None
    if isinstance(summary, dict) and summary.get("dropped_records") is not None:
        dropped = int(summary["dropped_records"])
    return {
        "cycles_in_file": int(cycles[-1]["cycle"]) if cycles else 0,
        "measured_cycles": len(measured),
        "first_measured_cycle": int(measured[0]["cycle"]),
        "missed_deadlines": missed,
        "skipped_periods": skipped,
        "dropped_records": dropped,
        "execution_ns": stats(execution),
        "wake_lateness_ns": stats(wake_lateness),
        "_values": {
            "execution_ns": execution,
            "wake_lateness_ns": wake_lateness,
        },
    }


def run_timeout_s(cycles: int, cycle_time_ns: int) -> float:
    """Returns a generous wall-clock timeout for one launcher run.

    The default loop sleeps with the platform's thread timer, and macOS can
    round a 1 ms sleep up to several milliseconds. The timeout therefore uses
    ten times the nominal duration plus a minute; that covers the observed
    macOS granularity without hiding a genuinely hung launcher.
    """
    expected_s = cycles * cycle_time_ns / 1_000_000_000
    return max(120.0, expected_s * 10.0 + 60.0)


def run_once(
    launcher: Path,
    cycle_time_ns: int,
    cycles: int,
    mode: str,
    run_index: int,
    out: Path,
    warmup_cycles: int,
) -> dict:
    """Runs the launcher once and returns its record."""
    run_dir = out / mode
    run_dir.mkdir(parents=True, exist_ok=True)
    metrics_path = run_dir / f"run-{run_index:02d}.jsonl" if mode == "rt" else None
    command = launcher_command(launcher, cycle_time_ns, cycles, mode, metrics_path)
    started_at = utc_now()
    started = time.perf_counter_ns()
    error = None
    return_code = None
    stdout = ""
    stderr = ""
    try:
        completed = subprocess.run(
            command,
            capture_output=True,
            text=True,
            timeout=run_timeout_s(cycles, cycle_time_ns),
            check=False,
        )
        return_code = completed.returncode
        stdout = completed.stdout
        stderr = completed.stderr
    except subprocess.TimeoutExpired as timeout:
        error = f"launcher timed out after {timeout.timeout:.1f}s"
        stdout = _decode(timeout.stdout)
        stderr = _decode(timeout.stderr)
    except OSError as os_error:
        error = f"cannot run the launcher: {os_error}"
    wall_time_ns = time.perf_counter_ns() - started

    record = {
        "run": run_index,
        "mode": mode,
        "command": command,
        "started_at": started_at,
        "finished_at": utc_now(),
        "wall_time_ns": wall_time_ns,
        "cycles": cycles,
        "return_code": return_code,
        "stdout_tail": _tail(stdout),
        "stderr_tail": _tail(stderr),
        "metrics_file": str(metrics_path) if metrics_path else None,
        "metrics_sha256": None,
        "header": None,
        "summary": None,
        "metrics": None,
        "wall_cycle_ns": {
            "mean": wall_time_ns // cycles if cycles else None,
        },
        "error": error,
    }
    if return_code not in (0, None) and error is None:
        record["error"] = f"launcher exited with status {return_code}"
    if record["error"] is None and mode == "rt":
        try:
            parsed = parse_metrics(metrics_path)
            metrics = analyze_metrics(parsed, warmup_cycles)
            record["header"] = parsed["header"]
            record["summary"] = parsed["summary"]
            record["metrics"] = metrics
            record["metrics_sha256"] = sha256_file(metrics_path)
        except (CompareError, OSError, ValueError) as metrics_error:
            record["error"] = str(metrics_error)
    return record


def _decode(value: bytes | str | None) -> str:
    if value is None:
        return ""
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace")
    return value


def _tail(text: str, limit: int = 2_000) -> str:
    return text[-limit:] if len(text) > limit else text


def aggregate_records(records: list[dict], mode: str) -> dict:
    """Combines all runs of one mode into the summary aggregate block."""
    wall_cycles: list[int] = []
    missed = 0
    skipped = 0
    dropped = 0
    dropped_known = False
    measured_cycles = 0
    execution: list[int] = []
    wake_lateness: list[int] = []
    for record in records:
        wall_cycle = record.get("wall_cycle_ns", {}).get("mean")
        if wall_cycle is not None:
            wall_cycles.append(int(wall_cycle))
        metrics = record.get("metrics")
        if not isinstance(metrics, dict):
            continue
        measured_cycles += int(metrics.get("measured_cycles", 0))
        missed += int(metrics.get("missed_deadlines", 0))
        skipped += int(metrics.get("skipped_periods", 0))
        if metrics.get("dropped_records") is not None:
            dropped += int(metrics["dropped_records"])
            dropped_known = True
        values = metrics.get("_values") or {}
        execution.extend(values.get("execution_ns", []))
        wake_lateness.extend(values.get("wake_lateness_ns", []))
    return {
        "runs": len(records),
        "measured_cycles": measured_cycles,
        "missed_deadlines": missed if mode == "rt" else None,
        "skipped_periods": skipped if mode == "rt" else None,
        "dropped_records": dropped if dropped_known else None,
        "execution_ns": stats(execution),
        "wake_lateness_ns": stats(wake_lateness),
        "wall_cycle_ns": stats(wall_cycles),
        "wall_time_ns": stats([int(record["wall_time_ns"]) for record in records]),
    }


def collect_mode(
    launcher: Path,
    cycle_time_ns: int,
    cycles: int,
    runs: int,
    mode: str,
    out: Path,
    warmup_cycles: int,
    build_form: str,
) -> dict:
    """Runs every repetition of one mode and writes its summary files."""
    out.mkdir(parents=True, exist_ok=True)
    host = collect_host_info()
    build_id, manifest = manifest_build_id(launcher)
    records: list[dict] = []
    errors: list[str] = []
    for index in range(1, runs + 1):
        record = run_once(
            launcher,
            cycle_time_ns,
            cycles,
            mode,
            index,
            out,
            warmup_cycles,
        )
        if record["error"] is not None:
            errors.append(f"run {index}: {record['error']}")
        records.append(record)
    aggregate = aggregate_records(records, mode)
    for record in records:
        metrics = record.get("metrics")
        if isinstance(metrics, dict):
            metrics.pop("_values", None)
    summary = {
        "schema": SUMMARY_SCHEMA,
        "generated_at": utc_now(),
        "mode": mode,
        "build_form": build_form,
        "launcher": {
            "path": str(launcher.resolve()),
            "sha256": sha256_file(launcher),
            "build_id": build_id,
            "manifest": manifest,
        },
        "host": host,
        "parameters": {
            "cycle_time_ns": cycle_time_ns,
            "cycles_per_run": cycles,
            "run_count": runs,
            "warmup_cycles": warmup_cycles,
        },
        "runs": records,
        "aggregate": aggregate,
        "result": "failed" if errors else "passed",
        "errors": errors,
        "notes": [
            "The baseline run loop does not emit stc-rt-metrics/1 because the "
            "stream is opt-in with --rt (D-249); baseline is compared by "
            "wall-clock cycle time.",
        ]
        if mode == "baseline"
        else [],
    }
    write_summary_files(out, summary)
    return summary


def render_markdown(summary: dict) -> str:
    """Renders the stable Markdown table for one mode."""
    host = summary["host"]
    parameters = summary["parameters"]
    aggregate = summary["aggregate"]
    lines = [
        f"# Scheduling comparison — {summary['mode']}",
        "",
        "| Item | Value |",
        "| --- | --- |",
        f"| Generated | {summary['generated_at']} |",
        f"| Mode | `{summary['mode']}` |",
        f"| Build form | {summary['build_form']} |",
        f"| Launcher | `{summary['launcher']['path']}` |",
        f"| Launcher SHA-256 | {summary['launcher']['sha256'] or 'n/a'} |",
        f"| Launcher build ID | {summary['launcher']['build_id'] or 'n/a'} |",
        f"| Host | {host.get('hostname') or 'n/a'} |",
        f"| Platform | {host.get('platform') or 'n/a'} |",
        f"| CPU | {host.get('cpu_model') or host.get('processor') or 'n/a'} |",
        f"| CPU count | {host.get('cpu_count') or 'n/a'} |",
        f"| Cycle time | {format_ns(parameters['cycle_time_ns'])} |",
        f"| Cycles per run | {parameters['cycles_per_run']} |",
        f"| Runs | {parameters['run_count']} |",
        f"| Warm-up discarded | {parameters['warmup_cycles']} cycles |",
        "",
        "## Per-run",
        "",
        "| Run | Exit | Wall (ms) | Measured | Exec p50 | Exec p99 | Wake p50 | "
        "Wake p99 | Missed | Skipped | Dropped | Sched |",
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |",
    ]
    for record in summary["runs"]:
        metrics = record.get("metrics") or {}
        execution = metrics.get("execution_ns") or {}
        wake = metrics.get("wake_lateness_ns") or {}
        header = record.get("header") or {}
        lines.append(
            "| {run} | {exit} | {wall:.3f} | {measured} | {exec_p50} | {exec_p99} | "
            "{wake_p50} | {wake_p99} | {missed} | {skipped} | {dropped} | {sched} |".format(
                run=record["run"],
                exit=record.get("return_code")
                if record.get("return_code") is not None
                else "n/a",
                wall=record["wall_time_ns"] / 1_000_000,
                measured=metrics.get("measured_cycles", "n/a"),
                exec_p50=format_ns(execution.get("p50")),
                exec_p99=format_ns(execution.get("p99")),
                wake_p50=format_ns(wake.get("p50")),
                wake_p99=format_ns(wake.get("p99")),
                missed=metrics.get("missed_deadlines", "n/a"),
                skipped=metrics.get("skipped_periods", "n/a"),
                dropped=metrics.get("dropped_records")
                if metrics.get("dropped_records") is not None
                else "n/a",
                sched=header.get("sched", "n/a"),
            )
        )
    lines += [
        "",
        "## Aggregate",
        "",
        "| Metric | min | p50 | p90 | p99 | p99.9 | max |",
        "| --- | --- | --- | --- | --- | --- | --- |",
    ]
    for name, key in (
        ("Execution", "execution_ns"),
        ("Wake lateness", "wake_lateness_ns"),
        ("Wall cycle", "wall_cycle_ns"),
        ("Wall time", "wall_time_ns"),
    ):
        block = aggregate.get(key) or {}
        lines.append(
            f"| {name} | {format_ns(block.get('min'))} | {format_ns(block.get('p50'))} | "
            f"{format_ns(block.get('p90'))} | {format_ns(block.get('p99'))} | "
            f"{format_ns(block.get('p99_9'))} | {format_ns(block.get('max'))} |"
        )
    lines += [
        "",
        f"- Measured cycles: {aggregate.get('measured_cycles', 0)}",
        f"- Missed deadlines: {_value(aggregate.get('missed_deadlines'))}",
        f"- Skipped periods: {_value(aggregate.get('skipped_periods'))}",
        f"- Dropped records: {_value(aggregate.get('dropped_records'))}",
        f"- Result: **{summary['result']}**",
    ]
    for note in summary.get("notes", []):
        lines.append(f"- Note: {note}")
    for error in summary.get("errors", []):
        lines.append(f"- Error: {error}")
    lines.append("")
    return "\n".join(lines)


def _value(value: object) -> str:
    return "n/a" if value is None else str(value)


def write_summary_files(out: Path, summary: dict) -> None:
    """Writes the stable JSON summary and the Markdown table."""
    (out / "summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    (out / "summary.md").write_text(render_markdown(summary), encoding="utf-8")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--launcher", required=True, help="path to the built launcher")
    parser.add_argument(
        "--cycle-time",
        default=DEFAULT_CYCLE_TIME,
        help=f"cycle time literal (default: {DEFAULT_CYCLE_TIME})",
    )
    parser.add_argument(
        "--cycles",
        type=int,
        default=DEFAULT_CYCLES,
        help=f"cycles per run (default: {DEFAULT_CYCLES})",
    )
    parser.add_argument(
        "--runs",
        type=int,
        default=DEFAULT_RUNS,
        help=f"runs per mode (default: {DEFAULT_RUNS})",
    )
    parser.add_argument(
        "--mode",
        choices=MODES,
        required=True,
        help="baseline (default loop) or rt (--rt with metrics)",
    )
    parser.add_argument("--out", required=True, help="output directory")
    parser.add_argument(
        "--warmup-cycles",
        type=int,
        default=DEFAULT_WARMUP_CYCLES,
        help=f"cycles discarded from the start (default: {DEFAULT_WARMUP_CYCLES})",
    )
    parser.add_argument(
        "--build-form",
        default=os.environ.get("STC_BUILD_FORM", "unspecified"),
        help="build-form label recorded in the summary (default: $STC_BUILD_FORM)",
    )
    return parser


def validate_args(args: argparse.Namespace) -> None:
    launcher = Path(args.launcher)
    if not launcher.is_file():
        raise CompareError(f"launcher not found: {launcher}")
    if args.cycles <= 0:
        raise CompareError("--cycles must be positive")
    if args.runs <= 0:
        raise CompareError("--runs must be positive")
    if args.warmup_cycles < 0:
        raise CompareError("--warmup-cycles must not be negative")
    if args.warmup_cycles >= args.cycles:
        raise CompareError("--warmup-cycles must be smaller than --cycles")
    try:
        parse_duration_ns(args.cycle_time)
    except ValueError as error:
        raise CompareError(str(error)) from error
    if not args.build_form.strip():
        raise CompareError("--build-form must not be empty")


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        validate_args(args)
        summary = collect_mode(
            Path(args.launcher),
            parse_duration_ns(args.cycle_time),
            args.cycles,
            args.runs,
            args.mode,
            Path(args.out),
            args.warmup_cycles,
            args.build_form.strip(),
        )
    except (CompareError, OSError, ValueError) as error:
        print(f"sched_compare: {error}", file=sys.stderr)
        return 2
    aggregate = summary["aggregate"]
    if summary["mode"] == "rt":
        detail = f"{aggregate['measured_cycles']} measured cycles"
    else:
        detail = (
            "baseline wall cycle p50 "
            f"{format_ns(aggregate['wall_cycle_ns']['p50'])}"
        )
    print(
        f"sched_compare: {summary['mode']} -> {args.out} "
        f"({detail}, {summary['result']})"
    )
    return 0 if summary["result"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
